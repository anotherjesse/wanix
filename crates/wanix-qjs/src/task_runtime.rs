use std::fmt;
use std::time::Duration;

use rust_wasi_quickjs::QuickJsRuntime;
use wanix_fs::{FsError, FsResult};
use wanix_task::Task;

use crate::host_api::qjs_error;
use crate::task_context::WanixExitState;
use crate::task_runtime_attach::set_task_interrupt_handler;
use event_loop::{drain_event_loop_if_running, finish_eval_with_event_loop_limits};

mod event_loop;

/// A live QuickJS runtime attached to a Wanix task.
///
/// The VM image can be snapshotted through [`Self::snapshot_bytes`] and later
/// restored through [`crate::QuickJsRunner::restore_task_runtime_from_bytes`]. The
/// snapshot contains only QuickJS/Wasm memory; task identity, namespace, fds,
/// env/cwd/cmd metadata, and exit state are reattached from the supplied Wanix
/// task each time a task runtime is created or restored.
pub struct QuickJsTaskRuntime {
    runtime: QuickJsRuntime,
    task: Task,
    exit_state: WanixExitState,
}

impl fmt::Debug for QuickJsTaskRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuickJsTaskRuntime")
            .field("task", &self.task)
            .field("exit_code", &self.exit_code().ok().flatten())
            .finish_non_exhaustive()
    }
}

impl QuickJsTaskRuntime {
    pub(crate) fn new(runtime: QuickJsRuntime, task: Task, exit_state: WanixExitState) -> Self {
        Self {
            runtime,
            task,
            exit_state,
        }
    }

    /// Returns the underlying QuickJS runtime.
    #[must_use]
    pub fn runtime(&self) -> &QuickJsRuntime {
        &self.runtime
    }

    /// Returns mutable access to the underlying QuickJS runtime.
    #[must_use]
    pub fn runtime_mut(&mut self) -> &mut QuickJsRuntime {
        &mut self.runtime
    }

    /// Evaluates JavaScript as a script under this task's Wanix process state.
    ///
    /// Immediate event-loop work is drained with the same bounded policy as
    /// [`QuickJsRunner::run_task`]. If JavaScript requests process exit through
    /// WASI `proc_exit`, the non-returning trap is treated as a successful task
    /// exit request.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS evaluation fails for a reason
    /// other than a requested Wanix process exit, or when pending jobs fail.
    pub fn eval_discard(&mut self, source: &str) -> FsResult<()> {
        self.eval_discard_with_event_loop_limits(source, Duration::ZERO, 1)
    }

    /// Evaluates JavaScript as a script with bounded post-eval event-loop work.
    ///
    /// `event_loop_wait_budget` controls how long the runtime may wait for
    /// future timers. `ready_io_turns` controls how many nonblocking stdlib fd
    /// handler turns run after evaluation. This is host lifecycle policy, not
    /// serialized Wanix task state.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS evaluation fails for a reason
    /// other than a requested Wanix process exit, or when bounded event-loop
    /// work fails.
    pub fn eval_discard_with_event_loop_limits(
        &mut self,
        source: &str,
        event_loop_wait_budget: Duration,
        ready_io_turns: usize,
    ) -> FsResult<()> {
        let result = self.runtime.eval_discard(source).map_err(qjs_error);
        finish_eval_with_event_loop_limits(
            &mut self.runtime,
            &self.exit_state,
            result,
            event_loop_wait_budget,
            ready_io_turns,
        )
    }

    /// Evaluates JavaScript as an ES module under this task's Wanix process state.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when module evaluation fails for a reason
    /// other than a requested Wanix process exit, or when pending jobs fail.
    pub fn eval_module_discard(&mut self, source: &str, filename: &str) -> FsResult<()> {
        self.eval_module_discard_with_event_loop_limits(source, filename, Duration::ZERO, 1)
    }

    /// Evaluates JavaScript as an ES module with bounded post-eval event-loop work.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when module evaluation fails for a reason
    /// other than a requested Wanix process exit, or when bounded event-loop
    /// work fails.
    pub fn eval_module_discard_with_event_loop_limits(
        &mut self,
        source: &str,
        filename: &str,
        event_loop_wait_budget: Duration,
        ready_io_turns: usize,
    ) -> FsResult<()> {
        let result = self
            .runtime
            .eval_module_discard(source, filename)
            .map_err(qjs_error);
        finish_eval_with_event_loop_limits(
            &mut self.runtime,
            &self.exit_state,
            result,
            event_loop_wait_budget,
            ready_io_turns,
        )
    }

    /// Captures the QuickJS VM image as serialized snapshot bytes.
    ///
    /// The returned bytes do not include Wanix task metadata or host resources.
    /// Restore them with [`crate::QuickJsRunner::restore_task_runtime_from_bytes`] and
    /// provide the Wanix task whose live resources should be reattached.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the underlying runtime cannot snapshot
    /// or serialize its VM image.
    pub fn snapshot_bytes(&mut self) -> FsResult<Vec<u8>> {
        if self.exit_state.code()?.is_some() {
            return Err(FsError::Other(
                "cannot snapshot after Wanix process exit".to_owned(),
            ));
        }
        self.runtime
            .snapshot()
            .and_then(|snapshot| snapshot.try_to_bytes())
            .map_err(qjs_error)
    }

    /// Sets the QuickJS heap allocation limit for this attached task runtime.
    ///
    /// The limit is host policy, not serialized Wanix task state. Callers that
    /// restore a VM image must reapply it to the restored runtime when they need
    /// the same policy after restore.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the underlying QuickJS runtime cannot
    /// apply the memory limit.
    pub fn set_memory_limit(&mut self, bytes: u32) -> FsResult<()> {
        self.runtime.set_memory_limit(bytes).map_err(qjs_error)
    }

    /// Sets the maximum number of QuickJS interrupt polls allowed during eval.
    ///
    /// The budget is host policy, not serialized Wanix task state. Callers that
    /// restore a VM image must reapply it to the restored runtime when they need
    /// the same policy after restore. The installed handler preserves Wanix
    /// process-exit interruption while also stopping CPU-bound JavaScript once
    /// the poll budget is exhausted.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the underlying QuickJS runtime cannot
    /// install the interrupt handler.
    pub fn set_interrupt_poll_budget(&mut self, polls: usize) -> FsResult<()> {
        set_task_interrupt_handler(&mut self.runtime, self.exit_state.clone(), Some(polls))
    }

    /// Runs bounded nonblocking ready-fd handler turns while the task VM is live.
    ///
    /// This is the task-runtime counterpart of QuickJS `qjs:os` fd readiness
    /// polling. It lets a composition layer feed task fds after initial script
    /// evaluation, then explicitly pump read/write handlers without exposing
    /// engine-specific errors or raw runtime plumbing.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when a QuickJS readiness turn fails, unless
    /// the failure is due to a requested Wanix process exit.
    pub fn run_ready_io_turns(&mut self, turns: usize) -> FsResult<()> {
        drain_event_loop_if_running(&mut self.runtime, &self.exit_state, Duration::ZERO, turns)
    }

    /// Runs bounded event-loop work for an already evaluated task runtime.
    ///
    /// This is host lifecycle policy for interactive composition layers. It
    /// lets a caller briefly wait for future timers and then run nonblocking
    /// ready-fd turns without evaluating more guest source.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS event-loop work fails, unless
    /// the failure is due to a requested Wanix process exit.
    pub fn run_event_loop_turns(
        &mut self,
        event_loop_wait_budget: Duration,
        ready_io_turns: usize,
    ) -> FsResult<()> {
        drain_event_loop_if_running(
            &mut self.runtime,
            &self.exit_state,
            event_loop_wait_budget,
            ready_io_turns,
        )
    }

    /// Returns the requested Wanix process exit code, if JavaScript has exited.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the exit-state lock is poisoned.
    pub fn exit_code(&self) -> FsResult<Option<i32>> {
        self.exit_state.code()
    }

    /// Writes the observed process exit status back to the Wanix task.
    ///
    /// If JavaScript has not requested an exit status, this records `0`, matching
    /// [`QuickJsRunner::run_task`].
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if the task exit field cannot be updated.
    pub fn finish(&self) -> FsResult<()> {
        self.task
            .set_exit(self.exit_state.code()?.unwrap_or(0).to_string())
    }
}
