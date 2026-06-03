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
    ready_io_turns: usize,
    interrupt_poll_budget: Option<usize>,
}

impl QuickJsTaskDriver {
    /// Creates a task driver from a runner.
    #[must_use]
    pub fn new(runner: Arc<QuickJsRunner>) -> Self {
        Self {
            runner,
            event_loop_wait_budget: Duration::ZERO,
            ready_io_turns: 1,
            interrupt_poll_budget: None,
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

    /// Sets how many nonblocking ready-fd handler turns run after eval.
    ///
    /// QuickJS does not report whether a ready-IO turn was idle or ran a
    /// handler, so this is an explicit fixed turn count. The default is one,
    /// matching the first ready-fd lifecycle slice.
    #[must_use]
    pub fn with_ready_io_turns(mut self, turns: usize) -> Self {
        self.ready_io_turns = turns;
        self
    }

    /// Sets the maximum number of QuickJS interrupt polls allowed during eval.
    ///
    /// When the budget is exhausted, the interrupt handler asks QuickJS to stop
    /// the current script. This is a bounded task-driver policy for CPU-bound
    /// JavaScript, not a general Wanix cancellation or signal mechanism.
    #[must_use]
    pub fn with_interrupt_poll_budget(mut self, polls: usize) -> Self {
        self.interrupt_poll_budget = Some(polls);
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
        match self.runner.run_task_with_runtime_limits(
            task,
            self.event_loop_wait_budget,
            self.ready_io_turns,
            self.interrupt_poll_budget,
        ) {
            Ok(_) => Ok(()),
            Err(err) => {
                let _ = task.set_exit("1");
                Err(err)
            }
        }
    }
}
