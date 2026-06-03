//! Task model and `#task` filesystem for Rust Wanix.
//!
//! This crate owns task allocation through `#task/new/*`, task metadata files,
//! fd tables, driver selection, and per-task namespaces.

mod driver;
mod fd;
mod table;
mod task;
mod task_files;
mod task_fs;

#[cfg(test)]
mod tests;

pub use driver::{NoopDriver, TaskDriver};
pub use fd::{Fd, FdTable, OpenFile};
pub use table::TaskTable;
pub use task::{Task, TaskId, TaskSpec};
pub use task_fs::TaskFs;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix task model";
