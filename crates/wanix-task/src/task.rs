use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath};
use wanix_vfs::{BindOptions, Namespace};

use crate::cmd::parse_cmd_argv;

mod confine;
mod fd_ops;
mod kill;
mod state;
mod types;

pub use confine::CONFINED_RESOURCE_PATH;
pub use kill::{InterruptHook, KILLED_EXIT};
use state::TaskState;
pub use types::{TaskId, TaskSpec};

/// Worst-case wakeup interval for [`Task::wait_exit`]. `set_exit` notifies, so
/// this only bounds a missed notification; a timeout re-checks, it never
/// reports a phantom exit.
const EXIT_RECHECK_INTERVAL: Duration = Duration::from_millis(50);

/// One task's shared state: the mutable record plus the exit signal that wakes
/// blocked [`Task::wait_exit`] callers when an exit status is recorded.
struct TaskShared {
    state: Mutex<TaskState>,
    exit_signal: Condvar,
}

/// A task handle. Clones point at the same task state.
#[derive(Clone)]
pub struct Task {
    shared: Arc<TaskShared>,
}

impl Task {
    /// Creates a task with manual kind and no parent.
    #[must_use]
    pub fn new(id: TaskId, spec: TaskSpec, namespace: Namespace) -> Self {
        Self::with_state(TaskState::manual(id, spec, namespace))
    }

    pub(crate) fn allocated(
        id: TaskId,
        parent: Option<TaskId>,
        kind: impl Into<String>,
        namespace: Namespace,
    ) -> Self {
        Self::with_state(TaskState::allocated(id, parent, kind, namespace))
    }

    fn with_state(state: TaskState) -> Self {
        Self {
            shared: Arc::new(TaskShared {
                state: Mutex::new(state),
                exit_signal: Condvar::new(),
            }),
        }
    }

    /// Returns the task id.
    #[must_use]
    pub fn id(&self) -> TaskId {
        self.read_state(|state| state.id)
            .expect("task state lock should be readable")
    }

    /// Returns the parent task id.
    #[must_use]
    pub fn parent_id(&self) -> Option<TaskId> {
        self.read_state(|state| state.parent)
            .expect("task state lock should be readable")
    }

    /// Returns the task kind.
    #[must_use]
    pub fn kind(&self) -> String {
        self.read_state(|state| state.kind.clone())
            .expect("task state lock should be readable")
    }

    pub(crate) fn set_kind(&self, kind: impl Into<String>) -> FsResult<()> {
        self.write_state(|state| {
            state.kind = kind.into();
            Ok(())
        })
    }

    /// Returns the task spec.
    #[must_use]
    pub fn spec(&self) -> TaskSpec {
        self.read_state(|state| state.spec.clone())
            .expect("task state lock should be readable")
    }

    /// Replaces the explicit launch description.
    ///
    /// This does not mutate the raw `cmd`, `env`, or `dir` task fields. Those
    /// remain separately visible through `#task` for compatibility with
    /// file-oriented task control.
    pub fn set_spec(&self, spec: TaskSpec) -> FsResult<()> {
        self.write_state(|state| {
            state.spec = spec;
            Ok(())
        })
    }

    /// Returns a clone of the task namespace binding table.
    #[must_use]
    pub fn namespace(&self) -> Namespace {
        self.read_state(|state| state.namespace.clone())
            .expect("task state lock should be readable")
    }

    /// Adds a binding to this task's namespace.
    ///
    /// # Errors
    ///
    /// Returns filesystem errors from path validation or namespace binding.
    pub fn bind(
        &self,
        filesystem: Arc<dyn FileSystem>,
        source: impl AsRef<str>,
        destination: impl AsRef<str>,
        options: BindOptions,
    ) -> FsResult<()> {
        self.write_state(|state| {
            state
                .namespace
                .bind(filesystem, source, destination, options)
        })
    }

    /// Returns the raw command text.
    #[must_use]
    pub fn cmd(&self) -> String {
        self.read_state(|state| state.cmd.clone())
            .expect("task state lock should be readable")
    }

    /// Sets the raw command text.
    pub fn set_cmd(&self, cmd: impl Into<String>) -> FsResult<()> {
        let cmd = cmd.into();
        let cmd_argv = parse_cmd_argv(&cmd)?;
        self.write_state(|state| {
            state.cmd = cmd;
            state.cmd_argv = cmd_argv;
            Ok(())
        })
    }

    /// Returns the parsed command arguments from the raw `cmd` task field.
    #[must_use]
    pub fn cmd_argv(&self) -> Option<Vec<String>> {
        self.read_state(|state| state.cmd_argv.clone())
            .expect("task state lock should be readable")
    }

    /// Returns the raw environment lines.
    #[must_use]
    pub fn env(&self) -> Vec<String> {
        self.read_state(|state| state.env.clone())
            .expect("task state lock should be readable")
    }

    /// Replaces the environment from newline-separated `KEY=value` lines.
    pub fn set_env_lines(&self, env: impl AsRef<str>) -> FsResult<()> {
        let env = env.as_ref().trim();
        self.write_state(|state| {
            state.env = if env.is_empty() {
                Vec::new()
            } else {
                env.lines().map(str::to_owned).collect()
            };
            Ok(())
        })
    }

    /// Returns the current working directory.
    #[must_use]
    pub fn dir(&self) -> NormalizedPath {
        self.read_state(|state| state.dir.clone())
            .expect("task state lock should be readable")
    }

    /// Sets the current working directory.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if `dir` is not a normalized path.
    pub fn set_dir(&self, dir: impl AsRef<str>) -> FsResult<()> {
        let dir = NormalizedPath::new(dir)?;
        self.write_state(|state| {
            state.dir = dir;
            Ok(())
        })
    }

    /// Returns the exit status text.
    #[must_use]
    pub fn exit(&self) -> String {
        self.read_state(|state| state.exit.clone())
            .expect("task state lock should be readable")
    }

    /// [`Self::exit`] without the lock-health expectation. Detached-run
    /// cleanup uses it after a caught driver panic: the panic may have
    /// poisoned the state lock, and panicking again would skip the cleanup
    /// entirely (waiters then observe `wait_exit`'s poison error, not a hang).
    pub(crate) fn try_exit(&self) -> FsResult<String> {
        self.read_state(|state| state.exit.clone())
    }

    /// Atomically claims the task's one allowed start.
    ///
    /// Returns `Ok(true)` only for the first caller; a task must never run
    /// twice — two driver runs would share one fd table and race
    /// `close_all_fds`/`set_exit` against the live run.
    pub(crate) fn try_mark_started(&self) -> FsResult<bool> {
        self.write_state(|state| Ok(!std::mem::replace(&mut state.started, true)))
    }

    /// Sets the exit status text and wakes any [`Self::wait_exit`] callers.
    pub fn set_exit(&self, exit: impl Into<String>) -> FsResult<()> {
        self.write_state(|state| {
            state.exit = exit.into();
            Ok(())
        })?;
        self.shared.exit_signal.notify_all();
        Ok(())
    }

    /// Blocks until an exit status has been recorded and returns it.
    ///
    /// Parks on a condvar (no busy spin) until [`Self::set_exit`] records a
    /// non-empty status — a task that never starts, or whose driver never
    /// records an exit, blocks its waiters indefinitely, so detached starts
    /// must guarantee an exit is recorded (see `TaskTable::start_detached`).
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the task state lock is poisoned.
    pub fn wait_exit(&self) -> FsResult<String> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| FsError::Other("task state lock poisoned".to_owned()))?;
        loop {
            if !state.exit.is_empty() {
                return Ok(state.exit.clone());
            }
            let (next, _timed_out) = self
                .shared
                .exit_signal
                .wait_timeout(state, EXIT_RECHECK_INTERVAL)
                .map_err(|_| FsError::Other("task exit wait poisoned".to_owned()))?;
            state = next;
        }
    }

    /// Releases every open fd, emptying the task's fd table.
    ///
    /// A task driver calls this when a task finishes running, mirroring the
    /// Unix/Plan 9 semantic that an exited process holds no descriptors. It is
    /// what lets a pipeline's producer drop its `#pipe` writer when it finishes,
    /// so the consumer observes EOF (a pipe reports end-of-file once its last
    /// writer is gone). Anything that needs a task's output after it exits must
    /// read the fd's backing (a file, sink, or pipe), not the task's fd table.
    ///
    /// This is distinct from [`set_exit`](Self::set_exit), which only records the
    /// status and may be called mid-operation while fds are still in use.
    pub fn close_all_fds(&self) {
        let _ = self.write_state(|state| {
            state.fds = crate::FdTable::new();
            Ok(())
        });
    }

    fn read_state<T>(&self, f: impl FnOnce(&TaskState) -> T) -> FsResult<T> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| FsError::Other("task state lock poisoned".to_owned()))?;
        Ok(f(&state))
    }

    fn write_state<T>(&self, f: impl FnOnce(&mut TaskState) -> FsResult<T>) -> FsResult<T> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| FsError::Other("task state lock poisoned".to_owned()))?;
        f(&mut state)
    }
}
