use wanix_fs::FsResult;

use crate::Task;

/// Runtime driver for a task kind.
pub trait TaskDriver: Send + Sync {
    /// Returns whether this driver can auto-start a task.
    fn check(&self, _task: &Task) -> bool {
        false
    }

    /// Starts a task.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the driver cannot start the task.
    fn start(&self, task: &Task) -> FsResult<()>;
}

/// Driver used by tests and early allocation flows.
#[derive(Debug, Default)]
pub struct NoopDriver;

impl TaskDriver for NoopDriver {
    fn start(&self, _task: &Task) -> FsResult<()> {
        Ok(())
    }
}
