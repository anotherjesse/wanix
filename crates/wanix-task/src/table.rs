use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, RwLock};

use wanix_fs::{FsError, FsResult};
use wanix_vfs::{BindOptions, BindPosition, Namespace};

use crate::{NoopDriver, Task, TaskDriver, TaskFs, TaskId};

/// Shared task table and driver registry.
#[derive(Clone, Default)]
pub struct TaskTable {
    inner: Arc<RwLock<TaskTableInner>>,
}

#[derive(Default)]
struct TaskTableInner {
    next_id: u64,
    tasks: BTreeMap<TaskId, Task>,
    drivers: BTreeMap<String, Arc<dyn TaskDriver>>,
}

impl fmt::Debug for TaskTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.read() {
            Ok(inner) => f
                .debug_struct("TaskTable")
                .field("next_id", &inner.next_id)
                .field("tasks", &inner.tasks.keys().collect::<Vec<_>>())
                .field("drivers", &inner.drivers.keys().collect::<Vec<_>>())
                .finish(),
            Err(_) => f
                .debug_struct("TaskTable")
                .field("state", &"poisoned")
                .finish(),
        }
    }
}

impl TaskTable {
    /// Creates an empty task table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a concrete driver for `kind`.
    pub fn register_driver(
        &self,
        kind: impl Into<String>,
        driver: Arc<dyn TaskDriver>,
    ) -> FsResult<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| FsError::Other("task table lock poisoned".to_owned()))?;
        inner.drivers.insert(kind.into(), driver);
        Ok(())
    }

    /// Registers a no-op driver for `kind`.
    pub fn register_noop_driver(&self, kind: impl Into<String>) -> FsResult<()> {
        self.register_driver(kind, Arc::new(NoopDriver))
    }

    /// Returns registered driver kinds.
    #[must_use]
    pub fn driver_kinds(&self) -> Vec<String> {
        let mut kinds: Vec<_> = self
            .inner
            .read()
            .map(|inner| inner.drivers.keys().cloned().collect())
            .unwrap_or_default();
        kinds.insert(0, "auto".to_owned());
        kinds
    }

    /// Allocates a root task with an empty namespace.
    pub fn allocate_root(&self, kind: impl AsRef<str>) -> FsResult<Task> {
        self.allocate_root_with_namespace(kind, Namespace::new())
    }

    /// Allocates a root task with `namespace`.
    pub fn allocate_root_with_namespace(
        &self,
        kind: impl AsRef<str>,
        namespace: Namespace,
    ) -> FsResult<Task> {
        self.allocate_task(kind.as_ref(), None, namespace)
    }

    /// Allocates a child task by cloning the parent's namespace.
    pub fn allocate_child(&self, kind: impl AsRef<str>, parent: TaskId) -> FsResult<Task> {
        let parent_task = self.get(parent).ok_or(FsError::NotFound)?;
        self.allocate_child_of(kind, &parent_task)
    }

    /// Allocates a child task by cloning the parent's namespace.
    pub fn allocate_child_of(&self, kind: impl AsRef<str>, parent: &Task) -> FsResult<Task> {
        self.allocate_task(kind.as_ref(), Some(parent.id()), parent.namespace())
    }

    /// Returns a task by id.
    #[must_use]
    pub fn get(&self, id: TaskId) -> Option<Task> {
        self.inner
            .read()
            .ok()
            .and_then(|inner| inner.tasks.get(&id).cloned())
    }

    /// Returns all tasks sorted by id.
    #[must_use]
    pub fn tasks(&self) -> Vec<Task> {
        self.inner
            .read()
            .map(|inner| inner.tasks.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Starts a task through its registered driver.
    ///
    /// A task starts at most once: a second `start` (or `start &`) is an
    /// honest error, never a re-run — two driver runs would share one fd
    /// table (interleaved stdio, double-consumed `#pipe` input) and race
    /// `close_all_fds`/`set_exit` against the live run.
    pub fn start(&self, id: TaskId) -> FsResult<()> {
        let task = self.get(id).ok_or(FsError::NotFound)?;
        claim_start(&task)?;
        self.run_driver(&task)
    }

    /// Resolves the task's driver and runs it. The caller must have claimed
    /// the task's one allowed start via [`claim_start`].
    fn run_driver(&self, task: &Task) -> FsResult<()> {
        let kind = task.kind();
        let drivers = {
            let inner = self
                .inner
                .read()
                .map_err(|_| FsError::Other("task table lock poisoned".to_owned()))?;
            inner
                .drivers
                .iter()
                .map(|(kind, driver)| (kind.clone(), Arc::clone(driver)))
                .collect::<Vec<_>>()
        };

        if kind == "auto" {
            let Some((kind, driver)) = drivers
                .into_iter()
                .find(|(_kind, driver)| driver.check(task))
            else {
                // No driver claims the program: starting is an honest error,
                // not a silent no-op (a silent Ok left waiters hanging and the
                // task's bound fds — e.g. a pipeline pipe writer — held forever).
                return Err(FsError::NotSupported);
            };
            task.set_kind(kind)?;
            return driver.start(task);
        }

        let driver = drivers
            .into_iter()
            .find_map(|(driver_kind, driver)| (driver_kind == kind).then_some(driver))
            .ok_or(FsError::NotFound)?;
        driver.start(task)
    }

    /// Starts a task on its own host OS thread and returns immediately.
    ///
    /// This is the ADR 0010 tier-2 executor shape: each running command task
    /// gets its own thread, so pipeline stages run concurrently and block only
    /// on their own I/O. The thread is detached — exit is observed through the
    /// task (`Task::wait_exit`, the `#task/<id>/wait` file), not a join handle.
    ///
    /// Waiters and pipe peers must never hang on a detached task: when the
    /// run finishes, the task's fds are released (idempotent next to the
    /// drivers' own `task-exit-closes-fds`, and the only release for a task no
    /// driver ran), and if no exit was recorded (`NoopDriver`, or a launch
    /// failure before the driver's own exit path) one is synthesized — `0` for
    /// a clean run, `127` for a task that could not start. The cleanup also
    /// survives a panicking driver (caught on the detached thread), and like
    /// [`Self::start`] a duplicate `start &` is rejected before any thread
    /// spawns.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the task does not exist, the task was
    /// already started, or the host thread cannot be spawned.
    pub fn start_detached(&self, id: TaskId) -> FsResult<()> {
        let task = self.get(id).ok_or(FsError::NotFound)?;
        claim_start(&task)?;
        let table = self.clone();
        let run_task = task.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("wanix-task-{}", id.get()))
            .spawn(move || {
                // Catch a panicking driver: an unwind that skipped the cleanup
                // below would hang waiters and pipe peers forever.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    table.run_driver(&run_task)
                }));
                finish_detached_run(&run_task, matches!(result, Ok(Ok(()))));
            });
        if let Err(err) = spawned {
            // The claimed start will never run: keep the no-hang guarantee.
            finish_detached_run(&task, false);
            return Err(FsError::Other(format!(
                "failed to spawn task thread: {err}"
            )));
        }
        Ok(())
    }

    /// Returns a `#task` filesystem view for `current`.
    #[must_use]
    pub fn filesystem_for(&self, current: TaskId) -> TaskFs {
        TaskFs::new(self.clone(), Some(current))
    }

    /// Returns a `#task` filesystem view without a current task.
    #[must_use]
    pub fn filesystem(&self) -> TaskFs {
        TaskFs::new(self.clone(), None)
    }

    fn allocate_task(
        &self,
        kind: &str,
        parent: Option<TaskId>,
        namespace: Namespace,
    ) -> FsResult<Task> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| FsError::Other("task table lock poisoned".to_owned()))?;
        if kind != "auto" && !inner.drivers.contains_key(kind) {
            return Err(FsError::NotFound);
        }

        inner.next_id += 1;
        let id = TaskId::new(inner.next_id);
        let mut namespace = namespace;
        namespace.bind(
            Arc::new(self.filesystem_for(id)),
            ".",
            "#task",
            BindOptions {
                position: BindPosition::Replace,
            },
        )?;
        let task = Task::allocated(id, parent, kind, namespace);
        inner.tasks.insert(id, task.clone());
        Ok(task)
    }
}

/// Claims the task's one allowed start; a duplicate start is an honest error.
fn claim_start(task: &Task) -> FsResult<()> {
    if task.try_mark_started()? {
        Ok(())
    } else {
        Err(FsError::Other("task already started".to_owned()))
    }
}

/// Finishes a detached run: releases the task's fds (so pipe peers observe
/// EOF/broken-pipe) and guarantees a recorded exit (`0` clean, `127`
/// otherwise) so waiters wake. Tolerates a state lock poisoned by a driver
/// panic — `wait_exit` then reports the poison error instead of hanging.
fn finish_detached_run(task: &Task, clean: bool) {
    task.close_all_fds();
    if matches!(task.try_exit(), Ok(exit) if exit.is_empty()) {
        let _ = task.set_exit(if clean { "0" } else { "127" });
    }
}
