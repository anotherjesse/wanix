use super::{ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, caller_memory};
use wasmtime::{Caller, Linker};

mod events;
mod layout;
mod readiness;
mod subscription;

pub(super) fn define_import(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "poll_oneoff", poll_oneoff)?;
    Ok(())
}

fn poll_oneoff(
    mut caller: Caller<'_, HostState>,
    in_ptr: i32,
    out_ptr: i32,
    nsubscriptions: i32,
    nevents_ptr: i32,
) -> wasmtime::Result<i32> {
    poll_oneoff_result(&mut caller, in_ptr, out_ptr, nsubscriptions, nevents_ptr)
        .map(preview1_poll_result)
}

fn poll_oneoff_result(
    caller: &mut Caller<'_, HostState>,
    in_ptr: i32,
    out_ptr: i32,
    nsubscriptions: i32,
    nevents_ptr: i32,
) -> wasmtime::Result<Result<(), i32>> {
    if nsubscriptions == 0 {
        return Ok(Err(ERRNO_INVAL));
    }
    let memory = caller_memory(caller)?;
    let subscriptions = subscription::read_poll_subscriptions(
        &memory,
        caller,
        in_ptr,
        out_ptr,
        nevents_ptr,
        nsubscriptions,
    )?;

    let events = readiness::collect_ready_events(caller, &subscriptions)?;
    events::write_poll_events(
        &memory,
        caller,
        out_ptr,
        nevents_ptr,
        &subscriptions,
        events,
        readiness::clock_event,
    )
}

fn preview1_poll_result(result: Result<(), i32>) -> i32 {
    match result {
        Ok(()) => ERRNO_SUCCESS,
        Err(errno) => errno,
    }
}
