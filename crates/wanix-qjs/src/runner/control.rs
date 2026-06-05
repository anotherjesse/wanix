use rust_wasi_quickjs::QuickJsRuntime;
use wanix_fs::FsResult;
use wanix_task::Fd;

use super::output::{OutputBuffer, RunControl};
use crate::host_api::qjs_error;
use crate::{define_task_output_callback, exit_requested_or_poisoned};

pub(super) fn configure_runtime_control(
    runtime: &mut QuickJsRuntime,
    control: &RunControl,
    stdout: &OutputBuffer,
    stderr: &OutputBuffer,
) -> FsResult<()> {
    if let Some(bytes) = control.memory_limit_bytes {
        runtime.set_memory_limit(bytes).map_err(qjs_error)?;
    }
    define_task_output_callback(
        runtime,
        "__wanix_stdout",
        stdout.clone(),
        control.exit_state.clone(),
        control.output_task.clone().map(|task| (task, Fd::STDOUT)),
    )?;
    define_task_output_callback(
        runtime,
        "__wanix_stderr",
        stderr.clone(),
        control.exit_state.clone(),
        control.output_task.clone().map(|task| (task, Fd::STDERR)),
    )?;
    configure_interrupt_handler(runtime, control)
}

fn configure_interrupt_handler(runtime: &mut QuickJsRuntime, control: &RunControl) -> FsResult<()> {
    if control.exit_state.is_none() && control.interrupt_poll_budget.is_none() {
        return Ok(());
    }

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
        .map_err(qjs_error)
}
