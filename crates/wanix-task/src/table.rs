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
    pub fn start(&self, id: TaskId) -> FsResult<()> {
        let (task, kind, drivers) = {
            let inner = self
                .inner
                .read()
                .map_err(|_| FsError::Other("task table lock poisoned".to_owned()))?;
            let task = inner.tasks.get(&id).cloned().ok_or(FsError::NotFound)?;
            let kind = task.kind();
            let drivers = inner
                .drivers
                .iter()
                .map(|(kind, driver)| (kind.clone(), Arc::clone(driver)))
                .collect::<Vec<_>>();
            (task, kind, drivers)
        };

        if kind == "auto" {
            let Some((kind, driver)) = drivers
                .into_iter()
                .find(|(_kind, driver)| driver.check(&task))
            else {
                return Ok(());
            };
            task.set_kind(kind)?;
            return driver.start(&task);
        }

        let driver = drivers
            .into_iter()
            .find_map(|(driver_kind, driver)| (driver_kind == kind).then_some(driver))
            .ok_or(FsError::NotFound)?;
        driver.start(&task)
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
