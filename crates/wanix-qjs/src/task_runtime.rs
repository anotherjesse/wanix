use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_wasi_quickjs::{QuickJsCreateOptions, QuickJsRestoreOptions, QuickJsRuntime};
use wanix_fs::{FsError, FsResult};
use wanix_task::{Fd, Task};
use wanix_wasi::WasiConfig;

use crate::host_api::{define_wanix_module_loader, define_wanix_task_globals, qjs_error};
use crate::task_command::{task_command, task_wasi_argv};
use crate::task_context::{WanixExitState, WanixTaskContext};
use crate::task_stdio::task_wasi_config;
use crate::wasi_host::WanixQuickJsWasiHost;
use crate::{
    CONSOLE_PRELUDE, QuickJsRunner, captured_stdio_config_for_wasi, define_task_output_callback,
    drain_runtime_work, exit_requested, wanix_wasi_host_error,
};

/// A live QuickJS runtime attached to a Wanix task.
///
/// The VM image can be snapshotted through [`Self::snapshot_bytes`] and later
/// restored through [`QuickJsRunner::restore_task_runtime_from_bytes`]. The
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
    fn new(runtime: QuickJsRuntime, task: Task, exit_state: WanixExitState) -> Self {
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
        self.finish_eval_with_event_loop_limits(result, event_loop_wait_budget, ready_io_turns)
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
        self.finish_eval_with_event_loop_limits(result, event_loop_wait_budget, ready_io_turns)
    }

    /// Captures the QuickJS VM image as serialized snapshot bytes.
    ///
    /// The returned bytes do not include Wanix task metadata or host resources.
    /// Restore them with [`QuickJsRunner::restore_task_runtime_from_bytes`] and
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
        self.drain_event_loop_if_running(Duration::ZERO, turns)
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
        self.drain_event_loop_if_running(event_loop_wait_budget, ready_io_turns)
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

    fn finish_eval_with_event_loop_limits(
        &mut self,
        result: FsResult<()>,
        event_loop_wait_budget: Duration,
        ready_io_turns: usize,
    ) -> FsResult<()> {
        match result {
            Ok(()) => self.drain_event_loop_if_running(event_loop_wait_budget, ready_io_turns),
            Err(error) => {
                if self.exit_state.code()?.is_some() {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }

    fn drain_event_loop_if_running(
        &mut self,
        event_loop_wait_budget: Duration,
        ready_io_turns: usize,
    ) -> FsResult<()> {
        if exit_requested(&Some(self.exit_state.clone()))? {
            return Ok(());
        }
        let result = drain_runtime_work(
            &mut self.runtime,
            &Some(self.exit_state.clone()),
            event_loop_wait_budget,
            ready_io_turns,
        );
        match result {
            Ok(_) => Ok(()),
            Err(error) => {
                if self.exit_state.code()?.is_some() {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }
}

impl QuickJsRunner {
    /// Creates a QuickJS runtime attached to a Wanix task.
    ///
    /// This installs Wanix-backed WASI imports, task stdout/stderr callbacks,
    /// the namespace module loader, `scriptArgs`, and an interrupt handler for
    /// process exit. The task script is not evaluated by this method; callers
    /// can evaluate code, snapshot the VM, restore it with fresh task resources,
    /// and finally call [`QuickJsTaskRuntime::finish`].
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when task command metadata is invalid, the
    /// Wanix WASI host cannot be created, the QuickJS runtime cannot be
    /// instantiated, or task host callbacks cannot be attached.
    pub fn create_task_runtime(&self, task: &Task) -> FsResult<QuickJsTaskRuntime> {
        task_command(task)?;
        let exit_state = WanixExitState::default();
        let create_options = task_create_options(task_wasi_config(task), exit_state.clone())?;
        let mut runtime = self
            .module
            .create_runtime_with_options(create_options)
            .map_err(qjs_error)?;
        attach_task_host_state(&mut runtime, task, exit_state.clone())?;
        Ok(QuickJsTaskRuntime::new(runtime, task.clone(), exit_state))
    }

    /// Restores a QuickJS task runtime from VM snapshot bytes.
    ///
    /// The snapshot supplies only QuickJS/Wasm memory. The supplied task provides
    /// live Wanix namespace, fd, argv/env/cwd, task API, stdio, and exit
    /// resources for the restored runtime.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when task command metadata is invalid, the
    /// snapshot cannot be restored, the Wanix WASI host cannot be created, or
    /// task host callbacks cannot be reattached.
    pub fn restore_task_runtime_from_bytes(
        &self,
        task: &Task,
        bytes: &[u8],
    ) -> FsResult<QuickJsTaskRuntime> {
        task_command(task)?;
        let exit_state = WanixExitState::default();
        let restore_options = task_restore_options(task_wasi_config(task), exit_state.clone())?;
        let mut runtime = self
            .module
            .restore_runtime_from_bytes_with_options(bytes, restore_options)
            .map_err(qjs_error)?;
        attach_task_host_state(&mut runtime, task, exit_state.clone())?;
        Ok(QuickJsTaskRuntime::new(runtime, task.clone(), exit_state))
    }
}

pub(crate) fn task_create_options(
    wasi_config: WasiConfig,
    exit_state: WanixExitState,
) -> FsResult<QuickJsCreateOptions> {
    let host_config = captured_stdio_config_for_wasi(&wasi_config);
    let wasi_host = WanixQuickJsWasiHost::new_with_exit_state(wasi_config, exit_state)
        .map_err(wanix_wasi_host_error)?;
    Ok(QuickJsCreateOptions::new()
        .with_host_config(host_config)
        .with_wasi_host(wasi_host))
}

pub(crate) fn task_restore_options(
    wasi_config: WasiConfig,
    exit_state: WanixExitState,
) -> FsResult<QuickJsRestoreOptions> {
    let host_config = captured_stdio_config_for_wasi(&wasi_config);
    let wasi_host = WanixQuickJsWasiHost::new_with_exit_state(wasi_config, exit_state)
        .map_err(wanix_wasi_host_error)?;
    Ok(QuickJsRestoreOptions::new()
        .with_host_config(host_config)
        .with_wasi_host(wasi_host))
}

fn attach_task_host_state(
    runtime: &mut QuickJsRuntime,
    task: &Task,
    exit_state: WanixExitState,
) -> FsResult<()> {
    define_task_output_callback(
        runtime,
        "__wanix_stdout",
        Arc::new(Mutex::new(Vec::new())),
        Some(exit_state.clone()),
        Some((task.clone(), Fd::STDOUT)),
    )?;
    define_task_output_callback(
        runtime,
        "__wanix_stderr",
        Arc::new(Mutex::new(Vec::new())),
        Some(exit_state.clone()),
        Some((task.clone(), Fd::STDERR)),
    )?;
    set_task_interrupt_handler(runtime, exit_state.clone(), None)?;

    let namespace = task.namespace();
    define_wanix_module_loader(runtime, namespace.clone())?;
    let context = WanixTaskContext::new(task_wasi_argv(task));
    define_wanix_task_globals(runtime, context)?;
    runtime.eval_discard(CONSOLE_PRELUDE).map_err(qjs_error)
}

fn set_task_interrupt_handler(
    runtime: &mut QuickJsRuntime,
    exit_state: WanixExitState,
    interrupt_poll_budget: Option<usize>,
) -> FsResult<()> {
    let mut interrupt_polls = 0usize;
    runtime
        .set_interrupt_handler(move || {
            if exit_state.code().map(|code| code.is_some()).unwrap_or(true) {
                return true;
            }
            let Some(budget) = interrupt_poll_budget else {
                return false;
            };
            interrupt_polls = interrupt_polls.saturating_add(1);
            interrupt_polls > budget
        })
        .map_err(qjs_error)
}
