use std::sync::{Arc, Mutex};
use std::time::Duration;

use rust_wasi_quickjs::QuickJsRuntime;
use wanix_fs::FsResult;
use wanix_task::{Fd, Task};

use crate::fd_api::define_fd_output_callback;
use crate::host_api::{define_output_callback, define_output_callback_with_exit_state, qjs_error};
use crate::task_context::WanixExitState;

const EVENT_LOOP_DRAIN_JOB_LIMIT: usize = 1024;

pub(crate) fn define_task_output_callback(
    runtime: &mut QuickJsRuntime,
    name: &'static str,
    output: Arc<Mutex<Vec<u8>>>,
    exit_state: Option<WanixExitState>,
    output_fd: Option<(Task, Fd)>,
) -> FsResult<()> {
    match (exit_state, output_fd) {
        (Some(exit_state), Some((task, fd))) => {
            define_fd_output_callback(runtime, name, task, fd, exit_state)
        }
        (Some(exit_state), None) => {
            define_output_callback_with_exit_state(runtime, name, output, exit_state)
        }
        (None, _) => define_output_callback(runtime, name, output),
    }
}

pub(crate) fn exit_requested(exit_state: &Option<WanixExitState>) -> FsResult<bool> {
    match exit_state {
        Some(exit_state) => exit_state.code().map(|code| code.is_some()),
        None => Ok(false),
    }
}

pub(crate) fn exit_requested_or_poisoned(exit_state: &Option<WanixExitState>) -> bool {
    exit_requested(exit_state).unwrap_or(true)
}

pub(crate) fn drain_runtime_work(
    runtime: &mut QuickJsRuntime,
    exit_state: &Option<WanixExitState>,
    event_loop_wait_budget: Duration,
    ready_io_turns: usize,
) -> FsResult<()> {
    if exit_requested(exit_state)? {
        return Ok(());
    }
    drain_timer_work(runtime, event_loop_wait_budget)?;
    for _ in 0..ready_io_turns {
        if exit_requested(exit_state)? {
            return Ok(());
        }
        runtime
            .execute_ready_io_event_loop_once()
            .map_err(qjs_error)?;
        if exit_requested(exit_state)? {
            return Ok(());
        }
        drain_timer_work(runtime, event_loop_wait_budget)?;
    }
    Ok(())
}

fn drain_timer_work(
    runtime: &mut QuickJsRuntime,
    event_loop_wait_budget: Duration,
) -> FsResult<()> {
    if event_loop_wait_budget.is_zero() {
        runtime
            .execute_immediate_event_loop_with_limit(EVENT_LOOP_DRAIN_JOB_LIMIT)
            .map_err(qjs_error)?;
        return Ok(());
    }

    runtime
        .execute_event_loop_with_wait_budget(EVENT_LOOP_DRAIN_JOB_LIMIT, event_loop_wait_budget)
        .map(|_status| ())
        .map_err(qjs_error)
}
