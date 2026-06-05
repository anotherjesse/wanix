use wasmtime::Caller;

use super::events::{PollEvents, PollReadiness};
use super::layout;
use crate::host::{ERRNO_NOSYS, HostState, QuickJsWasiErrno};

mod clock;
mod fd;

pub(super) const EVENTTYPE_CLOCK: u8 = 0;
pub(super) const EVENTTYPE_FD_READ: u8 = 1;
pub(super) const EVENTTYPE_FD_WRITE: u8 = 2;

pub(super) fn collect_ready_events(
    caller: &Caller<'_, HostState>,
    subscriptions: &[layout::Subscription],
) -> wasmtime::Result<PollEvents> {
    let mut accumulator = super::events::PollEventAccumulator::new();
    for subscription in subscriptions {
        if let Err(errno) = accumulator.record(ready_event(caller, subscription)?) {
            return Ok(PollEvents::Unsupported(errno));
        }
    }
    Ok(accumulator.finish(subscriptions.len()))
}

fn ready_event(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> wasmtime::Result<PollReadiness> {
    match layout::subscription_tag(subscription) {
        EVENTTYPE_FD_READ => fd::fd_event(caller, subscription, EVENTTYPE_FD_READ),
        EVENTTYPE_FD_WRITE => fd::fd_event(caller, subscription, EVENTTYPE_FD_WRITE),
        EVENTTYPE_CLOCK => clock::clock_readiness(caller, subscription),
        _ => Ok(PollReadiness::Unsupported(ERRNO_NOSYS)),
    }
}

pub(super) fn clock_event(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> Result<layout::Event, i32> {
    clock::clock_event(caller, subscription)
}

pub(super) fn event_with_errno(
    subscription: &layout::Subscription,
    event_type: u8,
    errno: Option<QuickJsWasiErrno>,
) -> layout::Event {
    let mut event = layout::event_with_userdata(subscription);
    let errno = errno.map_or(0, |errno| errno.preview1_result() as u16);
    layout::set_event_errno(&mut event, errno);
    layout::set_event_type(&mut event, event_type);
    event
}
