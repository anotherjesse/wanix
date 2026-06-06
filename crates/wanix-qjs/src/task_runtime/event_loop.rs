use std::time::Duration;

use rust_wasi_quickjs::QuickJsRuntime;
use wanix_fs::FsResult;

use crate::runtime_control::{drain_runtime_work, exit_requested};
use crate::task_context::WanixExitState;

pub(super) fn finish_eval_with_event_loop_limits(
    runtime: &mut QuickJsRuntime,
    exit_state: &WanixExitState,
    result: FsResult<()>,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
) -> FsResult<()> {
    match result {
        Ok(()) => {
            drain_event_loop_if_running(runtime, exit_state, event_loop_wait_budget, ready_io_turns)
        }
        Err(error) => {
            if exit_state.code()?.is_some() {
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

pub(super) fn drain_event_loop_if_running(
    runtime: &mut QuickJsRuntime,
    exit_state: &WanixExitState,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
) -> FsResult<()> {
    if exit_requested(&Some(exit_state.clone()))? {
        return Ok(());
    }
    let result = drain_runtime_work(
        runtime,
        &Some(exit_state.clone()),
        event_loop_wait_budget,
        ready_io_turns,
    );
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            if exit_state.code()?.is_some() {
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}
