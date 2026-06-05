use std::sync::{Arc, Mutex};

use rust_wasi_quickjs::{QuickJsCreateOptions, QuickJsRestoreOptions, QuickJsRuntime};
use wanix_fs::FsResult;
use wanix_task::{Fd, Task};
use wanix_wasi::WasiConfig;

use crate::host_api::{define_wanix_module_loader, define_wanix_task_globals, qjs_error};
use crate::runtime_control::define_task_output_callback;
use crate::task_command::{task_command, task_wasi_argv};
use crate::task_context::{WanixExitState, WanixTaskContext};
use crate::task_runtime::QuickJsTaskRuntime;
use crate::task_stdio::task_wasi_config;
use crate::wasi_host::WanixQuickJsWasiHost;
use crate::{
    CONSOLE_PRELUDE, QuickJsRunner, captured_stdio_config_for_wasi, wanix_wasi_host_error,
};

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
        let runtime = self
            .module
            .create_runtime_with_options(create_options)
            .map_err(qjs_error)?;
        attach_task_runtime(runtime, task, exit_state)
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
        let runtime = self
            .module
            .restore_runtime_from_bytes_with_options(bytes, restore_options)
            .map_err(qjs_error)?;
        attach_task_runtime(runtime, task, exit_state)
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

fn attach_task_runtime(
    mut runtime: QuickJsRuntime,
    task: &Task,
    exit_state: WanixExitState,
) -> FsResult<QuickJsTaskRuntime> {
    attach_task_host_state(&mut runtime, task, exit_state.clone())?;
    Ok(QuickJsTaskRuntime::new(runtime, task.clone(), exit_state))
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

pub(crate) fn set_task_interrupt_handler(
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
