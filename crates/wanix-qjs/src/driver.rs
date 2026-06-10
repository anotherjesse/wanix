use std::sync::Arc;
use std::time::Duration;

use wanix_fs::FsResult;
use wanix_task::{Task, TaskDriver};

use crate::QuickJsRunner;
use crate::task_command::task_program_for_check;

/// Wanix task driver for QuickJS tasks.
///
/// Stdio reads are blocking (ADR 0010 tier 2, via `wanix-wasi`): a guest that
/// loops `os.read(0, …)` parks inside the host import until bytes arrive or
/// end-of-stream is decidable, so a resident qjs task can serve a `#pipe`- or
/// `#term`-backed stdin line stream indefinitely — run it detached
/// (`TaskTable::start_detached`) so the park owns its own host thread. The
/// default execution policy is deliberately unbounded
/// (`interrupt_poll_budget` and `memory_limit_bytes` default to none), so a
/// resident loop trips no limit by accident; bounding one is an explicit
/// choice via [`Self::with_interrupt_poll_budget`], which counts JavaScript
/// interrupt polls between reads — time parked inside a blocking read
/// consumes none. Snapshots are not reachable while the guest is parked in a
/// read.
///
/// `#task/<id>/ctl kill` has no interrupt seam here yet: qjs tasks share one
/// runner (one Wasmtime engine), so an engine-wide epoch interrupt would kill
/// every running qjs task — a killed running qjs task only carries the
/// observable `Task::kill_requested` flag until a per-task seam lands. The
/// wasm driver (one engine per run) implements kill today.
#[derive(Debug, Clone)]
pub struct QuickJsTaskDriver {
    runner: Arc<QuickJsRunner>,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
    interrupt_poll_budget: Option<usize>,
    memory_limit_bytes: Option<u32>,
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
            memory_limit_bytes: None,
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

    /// Sets the QuickJS heap allocation limit in bytes.
    ///
    /// This is a task-driver host policy for allocation-heavy JavaScript. The
    /// default is no limit.
    #[must_use]
    pub fn with_memory_limit_bytes(mut self, bytes: u32) -> Self {
        self.memory_limit_bytes = Some(bytes);
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
        let result = self.runner.run_task_with_runtime_limits(
            task,
            self.event_loop_wait_budget,
            self.ready_io_turns,
            self.interrupt_poll_budget,
            self.memory_limit_bytes,
        );
        // A host-level failure (script unreadable, an over-cap verb refused at
        // open, engine setup) must reach the operator: a detached start (`start
        // &`) reduces the Err to an exit code, so the task's own stderr is the
        // only honest surface. Report before the fds close below.
        if let Err(err) = &result {
            task.report_run_failure(err);
        }
        // A finished task releases its fds (`task-exit-closes-fds`, same as the
        // wasm driver): a qjs pipeline producer's `#pipe` writer drops here so
        // the consumer observes EOF instead of hanging.
        task.close_all_fds();
        match result {
            Ok(_) => Ok(()),
            Err(err) => {
                let _ = task.set_exit("1");
                Err(err)
            }
        }
    }
}
