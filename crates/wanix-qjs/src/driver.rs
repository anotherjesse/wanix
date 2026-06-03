use std::sync::Arc;

use wanix_fs::FsResult;
use wanix_task::{Task, TaskDriver};

use crate::QuickJsRunner;

/// Wanix task driver for QuickJS tasks.
#[derive(Debug, Clone)]
pub struct QuickJsTaskDriver {
    runner: Arc<QuickJsRunner>,
}

impl QuickJsTaskDriver {
    /// Creates a task driver from a runner.
    #[must_use]
    pub fn new(runner: Arc<QuickJsRunner>) -> Self {
        Self { runner }
    }

    /// Returns the shared runner.
    #[must_use]
    pub fn runner(&self) -> &Arc<QuickJsRunner> {
        &self.runner
    }
}

impl TaskDriver for QuickJsTaskDriver {
    fn check(&self, task: &Task) -> bool {
        task.cmd().ends_with(".js") || task.cmd().contains(".js ")
    }

    fn start(&self, task: &Task) -> FsResult<()> {
        match self.runner.run_task(task) {
            Ok(_) => Ok(()),
            Err(err) => {
                let _ = task.set_exit("1");
                Err(err)
            }
        }
    }
}
