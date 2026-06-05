use std::time::Duration;

use rust_wasi_quickjs::{QuickJsCreateOptions, QuickJsRuntime};
use wanix_fs::{FsError, FsResult};
use wanix_task::Task;

use crate::host_api::{
    define_wanix_module_loader, define_wanix_task_globals, qjs_error, read_namespace_file,
};
use crate::task_command::{task_command, task_wasi_argv};
use crate::task_context::{WanixExitState, WanixTaskContext};
use crate::task_stdio::task_wasi_config;
use crate::wasi_host::WanixQuickJsWasiHost;
use crate::{
    CONSOLE_PRELUDE, QuickJsRunner, QuickJsWanixConfig, captured_stdio_options,
    captured_stdio_options_with_wanix_wasi, drain_runtime_work, uses_module_syntax,
};
use control::configure_runtime_control;
use output::{RunControl, RunFailure, collect_run_output, new_output_buffers, write_task_output};

mod control;
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

        let (stdout, stderr) = new_output_buffers();
        let result = (|| -> FsResult<()> {
            configure_runtime_control(&mut runtime, &control, &stdout, &stderr)?;
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

        let output = collect_run_output(&mut runtime, stdout, stderr)?;
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
