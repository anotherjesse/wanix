use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{FileSystem, FsError, FsResult, NormalizedPath};
use wanix_vfs::{BindOptions, Namespace};

use crate::FdTable;
use crate::cmd::parse_cmd_argv;

mod fd_ops;
mod types;

pub use types::{TaskId, TaskSpec};

/// A task handle. Clones point at the same task state.
#[derive(Clone)]
pub struct Task {
    state: Arc<Mutex<TaskState>>,
}

struct TaskState {
    id: TaskId,
    parent: Option<TaskId>,
    kind: String,
    spec: TaskSpec,
    cmd: String,
    cmd_argv: Option<Vec<String>>,
    env: Vec<String>,
    dir: NormalizedPath,
    exit: String,
    namespace: Namespace,
    fds: FdTable,
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.state.lock() {
            Ok(state) => f
                .debug_struct("Task")
                .field("id", &state.id)
                .field("parent", &state.parent)
                .field("kind", &state.kind)
                .field("cmd", &state.cmd)
                .field("cmd_argv", &state.cmd_argv)
                .field("dir", &state.dir)
                .field("exit", &state.exit)
                .field("fds", &state.fds)
                .finish(),
            Err(_) => f.debug_struct("Task").field("state", &"poisoned").finish(),
        }
    }
}

impl Task {
    /// Creates a task with manual kind and no parent.
    #[must_use]
    pub fn new(id: TaskId, spec: TaskSpec, namespace: Namespace) -> Self {
        Self::with_state(TaskState {
            id,
            parent: None,
            kind: "manual".to_owned(),
            cmd: String::new(),
            cmd_argv: None,
            env: Vec::new(),
            dir: spec.cwd.clone(),
            exit: String::new(),
            spec,
            namespace,
            fds: FdTable::new(),
        })
    }

    pub(crate) fn allocated(
        id: TaskId,
        parent: Option<TaskId>,
        kind: impl Into<String>,
        namespace: Namespace,
    ) -> Self {
        Self::with_state(TaskState {
            id,
            parent,
            kind: kind.into(),
            spec: TaskSpec::unset(),
            cmd: String::new(),
            cmd_argv: None,
            env: Vec::new(),
            dir: NormalizedPath::new(".").expect("root path is valid"),
            exit: String::new(),
            namespace,
            fds: FdTable::new(),
        })
    }

    fn with_state(state: TaskState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
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

    /// Sets the exit status text.
    pub fn set_exit(&self, exit: impl Into<String>) -> FsResult<()> {
        self.write_state(|state| {
            state.exit = exit.into();
            Ok(())
        })
    }

    fn read_state<T>(&self, f: impl FnOnce(&TaskState) -> T) -> FsResult<T> {
        let state = self
            .state
            .lock()
            .map_err(|_| FsError::Other("task state lock poisoned".to_owned()))?;
        Ok(f(&state))
    }

    fn write_state<T>(&self, f: impl FnOnce(&mut TaskState) -> FsResult<T>) -> FsResult<T> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| FsError::Other("task state lock poisoned".to_owned()))?;
        f(&mut state)
    }
}
