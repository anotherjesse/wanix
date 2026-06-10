//! Task model and `#task` filesystem for Rust Wanix.
//!
//! This crate owns task allocation through `#task/new/*`, task metadata files,
//! fd tables, driver selection, and per-task namespaces.

mod cmd;
mod driver;
mod fd;
mod table;
mod task;
mod task_command;
mod task_files;
mod task_fs;

#[cfg(test)]
mod tests;

pub use cmd::quote_cmd_argv;
pub use driver::{NoopDriver, TaskDriver};
pub use fd::{Fd, FdTable, OpenFile};
pub use table::TaskTable;
pub use task::{CONFINED_RESOURCE_PATH, InterruptHook, KILLED_EXIT, Task, TaskId, TaskSpec};
pub use task_command::{
    TaskCommand, task_command, task_program_for_check, task_wasi_argv, task_wasi_cwd, task_wasi_env,
};
pub use task_fs::TaskFs;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix task model";
