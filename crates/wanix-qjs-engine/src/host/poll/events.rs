use crate::guest::guest_offset;
use crate::host::HostState;
use wasmtime::{Caller, Memory};

use super::{ERRNO_NOSYS, layout};

pub(super) enum PollEvents {
    Ready(Vec<layout::Event>),
    PendingFd,
    WaitForSingleClock,
    Unsupported(i32),
}

pub(super) enum PollReadiness {
    Ready(layout::Event),
    PendingFd,
    WaitingClock,
    Unsupported(i32),
}

pub(super) struct PollEventAccumulator {
    ready_events: Vec<layout::Event>,
    pending_fd: bool,
}

impl PollEventAccumulator {
    pub(super) fn new() -> Self {
        Self {
            ready_events: Vec::new(),
            pending_fd: false,
        }
    }

    pub(super) fn record(&mut self, readiness: PollReadiness) -> Result<(), i32> {
        match readiness {
            PollReadiness::Ready(event) => self.ready_events.push(event),
            PollReadiness::PendingFd => self.pending_fd = true,
            PollReadiness::WaitingClock => {}
            PollReadiness::Unsupported(errno) => return Err(errno),
        }
        Ok(())
    }

    pub(super) fn finish(self, subscription_count: usize) -> PollEvents {
        if !self.ready_events.is_empty() {
            return PollEvents::Ready(self.ready_events);
        }
        if self.pending_fd {
            return PollEvents::PendingFd;
        }
        if subscription_count == 1 {
            PollEvents::WaitForSingleClock
        } else {
            PollEvents::Unsupported(ERRNO_NOSYS)
        }
    }
}

pub(super) fn write_poll_events(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    out_ptr: i32,
    nevents_ptr: i32,
    subscriptions: &[layout::Subscription],
    events: PollEvents,
    clock_event: impl FnOnce(
        &Caller<'_, HostState>,
        &layout::Subscription,
    ) -> Result<layout::Event, i32>,
) -> wasmtime::Result<Result<(), i32>> {
    match events {
        PollEvents::Ready(events) => {
            write_ready_events(memory, caller, out_ptr, nevents_ptr, events)
        }
        PollEvents::PendingFd => write_no_events(memory, caller, nevents_ptr),
        PollEvents::WaitForSingleClock => write_single_clock_event(
            memory,
            caller,
            out_ptr,
            nevents_ptr,
            &subscriptions[0],
            clock_event,
        ),
        PollEvents::Unsupported(errno) => unsupported_poll_events(errno),
    }
}

fn write_ready_events(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    out_ptr: i32,
    nevents_ptr: i32,
    events: Vec<layout::Event>,
) -> wasmtime::Result<Result<(), i32>> {
    layout::write_events(memory, caller, out_ptr, &events)?;
    layout::write_nevents(memory, caller, nevents_ptr, events.len())?;
    Ok(Ok(()))
}

fn write_no_events(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    nevents_ptr: i32,
) -> wasmtime::Result<Result<(), i32>> {
    layout::write_nevents(memory, caller, nevents_ptr, 0)?;
    Ok(Ok(()))
}

fn write_single_clock_event(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    out_ptr: i32,
    nevents_ptr: i32,
    subscription: &layout::Subscription,
    clock_event: impl FnOnce(
        &Caller<'_, HostState>,
        &layout::Subscription,
    ) -> Result<layout::Event, i32>,
) -> wasmtime::Result<Result<(), i32>> {
    let event = match clock_event(&*caller, subscription) {
        Ok(event) => event,
        Err(errno) => return Ok(Err(errno)),
    };
    memory.write(&mut *caller, guest_offset(out_ptr), &event)?;
    layout::write_nevents(memory, caller, nevents_ptr, 1)?;
    Ok(Ok(()))
}

fn unsupported_poll_events(errno: i32) -> wasmtime::Result<Result<(), i32>> {
    Ok(Err(errno))
}
