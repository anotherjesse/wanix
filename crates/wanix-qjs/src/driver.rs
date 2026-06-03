use std::sync::Arc;
use std::time::Duration;

use wanix_fs::FsResult;
use wanix_task::{Task, TaskDriver};

use crate::{QuickJsRunner, task_program_for_check};

/// Wanix task driver for QuickJS tasks.
#[derive(Debug, Clone)]
pub struct QuickJsTaskDriver {
    runner: Arc<QuickJsRunner>,
    event_loop_wait_budget: Duration,
}

impl QuickJsTaskDriver {
    /// Creates a task driver from a runner.
    #[must_use]
    pub fn new(runner: Arc<QuickJsRunner>) -> Self {
        Self {
            runner,
            event_loop_wait_budget: Duration::ZERO,
        }
    }

    /// Sets how long the driver may wait for future QuickJS timers after eval.
    ///
    /// The default is zero, which preserves the nonblocking task-exit behavior
    /// and only drains work that is already due.
    #[must_use]
    pub fn with_event_loop_wait_budget(mut self, budget: Duration) -> Self {
        self.event_loop_wait_budget = budget;
        self
    }

    /// Returns the shared runner.
    #[must_use]
    pub fn runner(&self) -> &Arc<QuickJsRunner> {
        &self.runner
    }
}

impl TaskDriver for QuickJsTaskDriver {
    fn check(&self, task: &Task) -> bool {
        task_program_for_check(task).is_some_and(|program| program.ends_with(".js"))
    }

    fn start(&self, task: &Task) -> FsResult<()> {
        match self
            .runner
            .run_task_with_event_loop_wait_budget(task, self.event_loop_wait_budget)
        {
            Ok(_) => Ok(()),
            Err(err) => {
                let _ = task.set_exit("1");
                Err(err)
            }
        }
    }
}
