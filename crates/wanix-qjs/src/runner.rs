use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_wasi_quickjs::{QuickJsCreateOptions, QuickJsRuntime};
use wanix_fs::{FsError, FsResult};
use wanix_task::{Fd, Task};

use crate::host_api::{
    define_wanix_module_loader, define_wanix_task_globals, qjs_error, read_namespace_file,
    take_buffer,
};
use crate::task_command::{task_command, task_wasi_argv};
use crate::task_context::{WanixExitState, WanixTaskContext};
use crate::task_stdio::task_wasi_config;
use crate::wasi_host::WanixQuickJsWasiHost;
use crate::{
    CONSOLE_PRELUDE, QuickJsRunner, QuickJsWanixConfig, captured_stdio_options,
    captured_stdio_options_with_wanix_wasi, define_task_output_callback, drain_runtime_work,
    exit_requested_or_poisoned, uses_module_syntax,
};
use output::{RunControl, RunFailure, write_task_output};

mod output;

pub use output::RunOutput;

impl QuickJsRunner {
    /// Runs JavaScript source outside Chrome and captures `print`/`console` output.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS creation, host callback setup,
    /// or JavaScript evaluation fails.
    pub fn run_source(&self, source: &str) -> FsResult<RunOutput> {
        self.run_source_with_setup(source, captured_stdio_options(), |_| Ok(()))
            .map_err(|failure| failure.error)
    }

    /// Runs JavaScript source with Wanix-backed QuickJS configuration.
    ///
    /// Wanix-owned WASI settings are attached as a live host provider. ES module
    /// source is evaluated as a module so guest code can import `qjs:std`,
    /// `qjs:os`, or other Wanix namespace modules.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when WASI host setup, QuickJS setup,
    /// namespace callbacks, or JavaScript evaluation fails.
    pub fn run_source_with_wanix_config(
        &self,
        source: &str,
        config: QuickJsWanixConfig,
    ) -> FsResult<RunOutput> {
        let namespace = config.wasi().namespace().clone();
        let create_options = captured_stdio_options_with_wanix_wasi(config)?;
        let run_as_module = uses_module_syntax(source);
        self.run_with_setup(
            source,
            create_options,
            move |runtime| define_wanix_module_loader(runtime, namespace),
            |runtime, source| {
                if run_as_module {
                    runtime
                        .eval_module_discard(source, "wanix-source.mjs")
                        .map_err(qjs_error)
                } else {
                    runtime.eval_discard(source).map_err(qjs_error)
                }
            },
        )
        .map_err(|failure| failure.error)
    }

    fn run_source_with_setup(
        &self,
        source: &str,
        create_options: QuickJsCreateOptions,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        self.run_with_setup(source, create_options, setup, |runtime, source| {
            runtime.eval_discard(source).map_err(qjs_error)
        })
    }

    fn run_with_setup(
        &self,
        source: &str,
        create_options: QuickJsCreateOptions,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
        eval: impl FnOnce(&mut QuickJsRuntime, &str) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        self.run_with_setup_control(source, create_options, RunControl::default(), setup, eval)
    }

    fn run_with_setup_control(
        &self,
        source: &str,
        create_options: QuickJsCreateOptions,
        control: RunControl,
        setup: impl FnOnce(&mut QuickJsRuntime) -> FsResult<()>,
        eval: impl FnOnce(&mut QuickJsRuntime, &str) -> FsResult<()>,
    ) -> Result<RunOutput, RunFailure> {
        let mut runtime = self
            .module
            .create_runtime_with_options(create_options)
            .map_err(|err| RunFailure {
                error: qjs_error(err),
                output: RunOutput::empty(),
            })?;

        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let result = (|| -> FsResult<()> {
            if let Some(bytes) = control.memory_limit_bytes {
                runtime.set_memory_limit(bytes).map_err(qjs_error)?;
            }
            define_task_output_callback(
                &mut runtime,
                "__wanix_stdout",
                Arc::clone(&stdout),
                control.exit_state.clone(),
                control.output_task.clone().map(|task| (task, Fd::STDOUT)),
            )?;
            define_task_output_callback(
                &mut runtime,
                "__wanix_stderr",
                Arc::clone(&stderr),
                control.exit_state.clone(),
                control.output_task.clone().map(|task| (task, Fd::STDERR)),
            )?;
            if control.exit_state.is_some() || control.interrupt_poll_budget.is_some() {
                let exit_state = control.exit_state.clone();
                let interrupt_poll_budget = control.interrupt_poll_budget;
                let mut interrupt_polls = 0usize;
                runtime
                    .set_interrupt_handler(move || {
                        if exit_requested_or_poisoned(&exit_state) {
                            return true;
                        }
                        let Some(budget) = interrupt_poll_budget else {
                            return false;
                        };
                        interrupt_polls = interrupt_polls.saturating_add(1);
                        interrupt_polls > budget
                    })
                    .map_err(qjs_error)?;
            }
            setup(&mut runtime)?;
            runtime.eval_discard(CONSOLE_PRELUDE).map_err(qjs_error)?;
            eval(&mut runtime, source)?;
            drain_runtime_work(
                &mut runtime,
                &control.exit_state,
                control.event_loop_wait_budget,
                control.ready_io_turns,
            )?;
            Ok(())
        })();

        let mut stdout = take_buffer(stdout).map_err(|error| RunFailure {
            error,
            output: RunOutput::empty(),
        })?;
        stdout.extend_from_slice(&runtime.take_captured_stdout());
        let mut stderr = take_buffer(stderr).map_err(|error| RunFailure {
            error,
            output: RunOutput::empty(),
        })?;
        stderr.extend_from_slice(&runtime.take_captured_stderr());
        let output = RunOutput::new(stdout, stderr);
        match result {
            Ok(()) => Ok(output),
            Err(error) => Err(RunFailure { error, output }),
        }
    }

    /// Runs the script named in `task.cmd()` and writes output through task fds.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the command is missing, the script cannot
    /// be read, QuickJS fails, or task fd writes fail.
    pub fn run_task(&self, task: &Task) -> FsResult<RunOutput> {
        self.run_task_with_event_loop_wait_budget(task, Duration::ZERO)
    }

    /// Runs the script named in `task.cmd()` and waits for future timers.
    ///
    /// The wait budget is a bounded task-driver policy for QuickJS
    /// standard-library timers such as `qjs:os.setTimeout`. A zero budget
    /// preserves the default nonblocking behavior and only drains already-due
    /// work.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the command is missing, the script cannot
    /// be read, QuickJS fails, task fd writes fail, or the bounded event pump
    /// encounters a callback error.
    pub fn run_task_with_event_loop_wait_budget(
        &self,
        task: &Task,
        event_loop_wait_budget: Duration,
    ) -> FsResult<RunOutput> {
        self.run_task_with_event_loop_limits(task, event_loop_wait_budget, 1)
    }

    /// Runs the script named in `task.cmd()` with bounded event-loop limits.
    ///
    /// `ready_io_turns` is a fixed nonblocking turn count because QuickJS's
    /// stdlib fd poll hook cannot distinguish an idle poll from a successful
    /// callback.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the command is missing, the script cannot
    /// be read, QuickJS fails, task fd writes fail, or the bounded event pump
    /// encounters a callback error.
    pub fn run_task_with_event_loop_limits(
        &self,
        task: &Task,
        event_loop_wait_budget: Duration,
        ready_io_turns: usize,
    ) -> FsResult<RunOutput> {
        self.run_task_with_runtime_limits(task, event_loop_wait_budget, ready_io_turns, None, None)
    }

    /// Runs the script named in `task.cmd()` with bounded runtime limits.
    ///
    /// `ready_io_turns` is a fixed nonblocking turn count because QuickJS's
    /// stdlib fd poll hook cannot distinguish an idle poll from a successful
    /// callback. `interrupt_poll_budget` bounds CPU-bound eval by asking
    /// QuickJS to interrupt after the configured number of interrupt polls.
    /// `memory_limit_bytes` bounds QuickJS heap allocation.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the command is missing, the script cannot
    /// be read, QuickJS fails or is interrupted by policy, task fd writes fail,
    /// or the bounded event pump encounters a callback error.
    pub fn run_task_with_runtime_limits(
        &self,
        task: &Task,
        event_loop_wait_budget: Duration,
        ready_io_turns: usize,
        interrupt_poll_budget: Option<usize>,
        memory_limit_bytes: Option<u32>,
    ) -> FsResult<RunOutput> {
        let command = task_command(task)?;
        let script_path = command.program.clone();
        let script_filename = script_path.to_string();
        let namespace = task.namespace();
        let source = read_namespace_file(&namespace, &script_path)?;
        let host = QuickJsWanixConfig::new(task_wasi_config(task));
        let exit_state = WanixExitState::default();
        let wasi_host =
            WanixQuickJsWasiHost::new_with_exit_state(host.wasi().clone(), exit_state.clone())
                .map_err(|err| {
                    FsError::Other(format!(
                        "failed to create Wanix-backed QuickJS WASI host: {err:?}"
                    ))
                })?;
        let create_options = captured_stdio_options().with_wasi_host(wasi_host);
        let run_as_module = uses_module_syntax(&source);
        let script_args = task_wasi_argv(task);
        let context = WanixTaskContext::new(script_args);
        match self.run_with_setup_control(
            &source,
            create_options,
            RunControl {
                exit_state: Some(exit_state.clone()),
                output_task: Some(task.clone()),
                event_loop_wait_budget,
                ready_io_turns,
                interrupt_poll_budget,
                memory_limit_bytes,
            },
            move |runtime| {
                define_wanix_module_loader(runtime, namespace.clone())?;
                define_wanix_task_globals(runtime, context)
            },
            |runtime, source| {
                if run_as_module {
                    runtime
                        .eval_module_discard(source, &script_filename)
                        .map_err(qjs_error)
                } else {
                    runtime.eval_discard(source).map_err(qjs_error)
                }
            },
        ) {
            Ok(output) => {
                write_task_output(task, &output)?;
                let exit_code = exit_state.code()?.unwrap_or(0);
                task.set_exit(exit_code.to_string())?;
                Ok(output)
            }
            Err(failure) => {
                write_task_output(task, &failure.output)?;
                if let Some(exit_code) = exit_state.code()? {
                    task.set_exit(exit_code.to_string())?;
                    return Ok(failure.output);
                }
                Err(failure.error)
            }
        }
    }
}
