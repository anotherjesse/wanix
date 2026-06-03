use std::fmt;
use std::sync::{Arc, Mutex};

use rust_wasi_quickjs::{QuickJsCreateOptions, QuickJsRestoreOptions, QuickJsRuntime};
use wanix_fs::{FsError, FsResult};
use wanix_task::{Fd, Task};

use crate::host_api::{define_wanix_host_api, define_wanix_module_loader, qjs_error};
use crate::task_context::{WanixExitState, WanixTaskContext};
use crate::task_stdio::task_wasi_config;
use crate::wasi_host::WanixQuickJsWasiHost;
use crate::{
    CONSOLE_PRELUDE, QuickJsRunner, captured_stdio_config, define_task_output_callback,
    exit_requested, task_command, task_wasi_argv, wanix_wasi_host_error,
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
    /// Pending jobs are drained with the same bounded policy as
    /// [`QuickJsRunner::run_task`]. If JavaScript requests process exit through
    /// WASI `proc_exit`, the non-returning trap is treated as a successful task
    /// exit request.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when QuickJS evaluation fails for a reason
    /// other than a requested Wanix process exit, or when pending jobs fail.
    pub fn eval_discard(&mut self, source: &str) -> FsResult<()> {
        let result = self.runtime.eval_discard(source).map_err(qjs_error);
        self.finish_eval(result)
    }

    /// Evaluates JavaScript as an ES module under this task's Wanix process state.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when module evaluation fails for a reason
    /// other than a requested Wanix process exit, or when pending jobs fail.
    pub fn eval_module_discard(&mut self, source: &str, filename: &str) -> FsResult<()> {
        let result = self
            .runtime
            .eval_module_discard(source, filename)
            .map_err(qjs_error);
        self.finish_eval(result)
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

    fn finish_eval(&mut self, result: FsResult<()>) -> FsResult<()> {
        match result {
            Ok(()) => self.drain_pending_jobs_if_running(),
            Err(error) => {
                if self.exit_state.code()?.is_some() {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }

    fn drain_pending_jobs_if_running(&mut self) -> FsResult<()> {
        if exit_requested(&Some(self.exit_state.clone()))? {
            return Ok(());
        }
        let result = self
            .runtime
            .execute_pending_jobs_with_limit(1024)
            .map_err(qjs_error);
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
    /// the namespace module loader, the interim `Wanix` task API, and an
    /// interrupt handler for process exit. The task script is not evaluated by
    /// this method; callers can evaluate code, snapshot the VM, restore it with
    /// fresh task resources, and finally call [`QuickJsTaskRuntime::finish`].
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when task command metadata is invalid, the
    /// Wanix WASI host cannot be created, the QuickJS runtime cannot be
    /// instantiated, or task host callbacks cannot be attached.
    pub fn create_task_runtime(&self, task: &Task) -> FsResult<QuickJsTaskRuntime> {
        let command = task_command(task)?;
        let exit_state = WanixExitState::default();
        let wasi_host =
            WanixQuickJsWasiHost::new_with_exit_state(task_wasi_config(task), exit_state.clone())
                .map_err(wanix_wasi_host_error)?;
        let create_options = QuickJsCreateOptions::new()
            .with_host_config(captured_stdio_config())
            .with_wasi_host(wasi_host);
        let mut runtime = self
            .module
            .create_runtime_with_options(create_options)
            .map_err(qjs_error)?;
        attach_task_host_state(&mut runtime, task, command, exit_state.clone())?;
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
        let command = task_command(task)?;
        let exit_state = WanixExitState::default();
        let wasi_host =
            WanixQuickJsWasiHost::new_with_exit_state(task_wasi_config(task), exit_state.clone())
                .map_err(wanix_wasi_host_error)?;
        let restore_options = QuickJsRestoreOptions::new()
            .with_host_config(captured_stdio_config())
            .with_wasi_host(wasi_host);
        let mut runtime = self
            .module
            .restore_runtime_from_bytes_with_options(bytes, restore_options)
            .map_err(qjs_error)?;
        attach_task_host_state(&mut runtime, task, command, exit_state.clone())?;
        Ok(QuickJsTaskRuntime::new(runtime, task.clone(), exit_state))
    }
}

fn attach_task_host_state(
    runtime: &mut QuickJsRuntime,
    task: &Task,
    command: crate::TaskCommand,
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
    runtime
        .set_interrupt_handler({
            let exit_state = exit_state.clone();
            move || exit_state.code().map(|code| code.is_some()).unwrap_or(true)
        })
        .map_err(qjs_error)?;

    let namespace = task.namespace();
    define_wanix_module_loader(runtime, namespace.clone())?;
    let context = WanixTaskContext::new(task_wasi_argv(task), command.cwd);
    define_wanix_host_api(
        runtime,
        namespace,
        context,
        Some(exit_state),
        Some(task.clone()),
    )?;
    runtime.eval_discard(CONSOLE_PRELUDE).map_err(qjs_error)
}
