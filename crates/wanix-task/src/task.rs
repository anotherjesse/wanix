use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileSystem, FsError, FsResult, Metadata, NormalizedPath};
use wanix_vfs::{BindOptions, Namespace};

use crate::{Fd, FdTable};

/// Stable task identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(NonZeroU64);

impl TaskId {
    /// Creates a task id.
    ///
    /// # Panics
    ///
    /// Panics if `id` is zero. Root tasks should start at `1`.
    #[must_use]
    pub fn new(id: u64) -> Self {
        Self(NonZeroU64::new(id).expect("task id must be non-zero"))
    }

    /// Returns the numeric task id.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// Explicit task launch description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    /// Program path resolved inside the task namespace.
    pub program: NormalizedPath,
    /// Program arguments, excluding `program`.
    pub args: Vec<String>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
    /// Current directory inside the namespace.
    pub cwd: NormalizedPath,
}

impl TaskSpec {
    /// Creates a task spec with no args, no env, and root cwd.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `program` is not a normalized path.
    pub fn new(program: impl AsRef<str>) -> FsResult<Self> {
        Ok(Self {
            program: NormalizedPath::new(program)?,
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: NormalizedPath::new(".")?,
        })
    }

    pub(crate) fn unset() -> Self {
        Self::new(".").expect("root path is valid")
    }
}

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
        self.write_state(|state| {
            state.cmd = cmd.into();
            Ok(())
        })
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

    /// Stores an open file in this task's fd table.
    pub fn open_fd(&self, file: Box<dyn File>, path: NormalizedPath) -> FsResult<Fd> {
        self.write_state(|state| Ok(state.fds.open(file, path)))
    }

    /// Installs a specific fd in this task's fd table.
    pub fn insert_fd(&self, fd: Fd, file: Box<dyn File>, path: NormalizedPath) -> FsResult<()> {
        self.write_state(|state| {
            state.fds.insert_at(fd, file, path);
            Ok(())
        })
    }

    /// Closes an fd.
    pub fn close_fd(&self, fd: Fd) -> FsResult<()> {
        self.write_state(|state| state.fds.close(fd))
    }

    /// Reads from an fd.
    pub fn read_fd(&self, fd: Fd, buf: &mut [u8]) -> FsResult<usize> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.read(buf)
    }

    /// Writes to an fd.
    pub fn write_fd(&self, fd: Fd, buf: &[u8]) -> FsResult<usize> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.write(buf)
    }

    /// Returns metadata for an open fd.
    pub fn fd_metadata(&self, fd: Fd) -> FsResult<Metadata> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.metadata()
    }

    /// Returns the path associated with an open fd.
    pub fn fd_path(&self, fd: Fd) -> FsResult<NormalizedPath> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        Ok(file.path().clone())
    }

    /// Returns sorted open fd numbers.
    #[must_use]
    pub fn fd_numbers(&self) -> Vec<Fd> {
        self.read_state(|state| state.fds.fds())
            .expect("task state lock should be readable")
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
