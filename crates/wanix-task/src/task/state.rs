use std::fmt;

use wanix_fs::NormalizedPath;
use wanix_vfs::Namespace;

use crate::FdTable;

use super::{Task, TaskId, TaskSpec};

pub(super) struct TaskState {
    pub(super) id: TaskId,
    pub(super) parent: Option<TaskId>,
    pub(super) kind: String,
    pub(super) spec: TaskSpec,
    pub(super) cmd: String,
    pub(super) cmd_argv: Option<Vec<String>>,
    pub(super) env: Vec<String>,
    pub(super) dir: NormalizedPath,
    pub(super) exit: String,
    /// Whether a start has been accepted: a task runs at most once.
    pub(super) started: bool,
    /// Whether a kill has been requested (see [`Task::kill`]).
    pub(super) kill_requested: bool,
    /// The running driver's armed guest interrupter, if any.
    pub(super) interrupt_hook: Option<super::InterruptHook>,
    pub(super) namespace: Namespace,
    pub(super) fds: FdTable,
}

impl TaskState {
    pub(super) fn manual(id: TaskId, spec: TaskSpec, namespace: Namespace) -> Self {
        Self {
            id,
            parent: None,
            kind: "manual".to_owned(),
            cmd: String::new(),
            cmd_argv: None,
            env: Vec::new(),
            dir: spec.cwd.clone(),
            exit: String::new(),
            started: false,
            kill_requested: false,
            interrupt_hook: None,
            spec,
            namespace,
            fds: FdTable::new(),
        }
    }

    pub(super) fn allocated(
        id: TaskId,
        parent: Option<TaskId>,
        kind: impl Into<String>,
        namespace: Namespace,
    ) -> Self {
        Self {
            id,
            parent,
            kind: kind.into(),
            spec: TaskSpec::unset(),
            cmd: String::new(),
            cmd_argv: None,
            env: Vec::new(),
            dir: NormalizedPath::new(".").expect("root path is valid"),
            exit: String::new(),
            started: false,
            kill_requested: false,
            interrupt_hook: None,
            namespace,
            fds: FdTable::new(),
        }
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.shared.state.lock() {
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
