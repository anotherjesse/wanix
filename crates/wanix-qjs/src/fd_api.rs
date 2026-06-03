use anyhow::{anyhow, bail};
use rust_wasi_quickjs::{QuickJsHostValue, QuickJsRuntime};
use wanix_fs::FsResult;
use wanix_task::{Fd, Task};

use crate::{host_api::display_host_value, host_api::qjs_error, task_context::WanixExitState};

pub(crate) fn define_fd_output_callback(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    task: Task,
    fd: Fd,
    exit_state: WanixExitState,
) -> FsResult<()> {
    runtime
        .define_global_host_function(name, move |args| {
            if exit_state.is_requested()? {
                return Ok(QuickJsHostValue::Undefined);
            }
            let text = args.iter().map(display_host_value).collect::<String>();
            write_all_fd(&task, fd, text.as_bytes(), name)?;
            Ok(QuickJsHostValue::Undefined)
        })
        .map_err(qjs_error)
}

fn write_all_fd(task: &Task, fd: Fd, bytes: &[u8], function: &str) -> anyhow::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let count = task
            .write_fd(fd, &bytes[written..])
            .map_err(|err| anyhow!("{function} failed to write fd {}: {err}", fd.get()))?;
        if count == 0 {
            bail!(
                "{function} failed to write fd {}: wrote zero bytes",
                fd.get()
            );
        }
        written += count;
    }
    Ok(())
}
