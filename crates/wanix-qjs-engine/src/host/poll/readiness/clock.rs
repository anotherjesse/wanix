use std::thread;
use std::time::Duration;

use wasmtime::Caller;

use super::{EVENTTYPE_CLOCK, event_with_errno};
use crate::host::poll::{events::PollReadiness, layout};
use crate::host::{ERRNO_INVAL, ERRNO_NOSYS, HostState};

const CLOCKID_REALTIME: u32 = 0;
const CLOCKID_MONOTONIC: u32 = 1;
const SUBCLOCKFLAGS_ABSTIME: u16 = 1 << 0;

pub(super) fn clock_readiness(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> wasmtime::Result<PollReadiness> {
    Ok(match immediate_clock_event(caller, subscription) {
        Ok(Some(event)) => PollReadiness::Ready(event),
        Ok(None) => PollReadiness::WaitingClock,
        Err(errno) => PollReadiness::Unsupported(errno),
    })
}

fn immediate_clock_event(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> Result<Option<layout::Event>, i32> {
    let sleep_ns = clock_sleep_ns(caller, subscription)?;
    if sleep_ns == 0 {
        Ok(Some(event_with_errno(subscription, EVENTTYPE_CLOCK, None)))
    } else {
        Ok(None)
    }
}

pub(super) fn clock_event(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> Result<layout::Event, i32> {
    let sleep_ns = clock_sleep_ns(caller, subscription)?;
    if sleep_ns != 0 {
        thread::sleep(Duration::from_nanos(sleep_ns));
    }
    Ok(event_with_errno(subscription, EVENTTYPE_CLOCK, None))
}

fn clock_sleep_ns(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> Result<u64, i32> {
    validate_clock_subscription(subscription)?;
    let timeout_ns = layout::subscription_clock_timeout(subscription);
    let flags = layout::subscription_clock_flags(subscription);
    Ok(clock_sleep_duration_ns(caller, timeout_ns, flags))
}

fn validate_clock_subscription(subscription: &layout::Subscription) -> Result<(), i32> {
    if layout::subscription_tag(subscription) != EVENTTYPE_CLOCK {
        return Err(ERRNO_NOSYS);
    }
    let clock_id = layout::subscription_clock_id(subscription);
    if clock_id != CLOCKID_REALTIME && clock_id != CLOCKID_MONOTONIC {
        return Err(ERRNO_NOSYS);
    }
    let flags = layout::subscription_clock_flags(subscription);
    if flags & !SUBCLOCKFLAGS_ABSTIME != 0 {
        return Err(ERRNO_INVAL);
    }
    Ok(())
}

fn clock_sleep_duration_ns(caller: &Caller<'_, HostState>, timeout_ns: u64, flags: u16) -> u64 {
    if flags & SUBCLOCKFLAGS_ABSTIME != 0 {
        timeout_ns.saturating_sub(caller.data().config().clock_time_ns())
    } else {
        timeout_ns
    }
}
