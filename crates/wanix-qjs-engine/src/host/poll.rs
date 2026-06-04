use std::thread;
use std::time::Duration;

use super::{ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, caller_memory};
use super::{QuickJsWasiErrno, QuickJsWasiFdStat};
use events::{PollEvents, PollReadiness};
use wasmtime::{Caller, Linker};

mod events;
mod layout;
mod subscription;

const EVENTTYPE_CLOCK: u8 = 0;
const EVENTTYPE_FD_READ: u8 = 1;
const EVENTTYPE_FD_WRITE: u8 = 2;
const CLOCKID_REALTIME: u32 = 0;
const CLOCKID_MONOTONIC: u32 = 1;
const SUBCLOCKFLAGS_ABSTIME: u16 = 1 << 0;
const RIGHT_FD_READ: u64 = 1 << 1;
const RIGHT_FD_WRITE: u64 = 1 << 6;

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

    let events = collect_ready_events(caller, &subscriptions)?;
    events::write_poll_events(
        &memory,
        caller,
        out_ptr,
        nevents_ptr,
        &subscriptions,
        events,
        clock_event,
    )
}

fn preview1_poll_result(result: Result<(), i32>) -> i32 {
    match result {
        Ok(()) => ERRNO_SUCCESS,
        Err(errno) => errno,
    }
}

fn collect_ready_events(
    caller: &Caller<'_, HostState>,
    subscriptions: &[layout::Subscription],
) -> wasmtime::Result<PollEvents> {
    let mut accumulator = events::PollEventAccumulator::new();
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
        EVENTTYPE_FD_READ => fd_event(caller, subscription, EVENTTYPE_FD_READ, RIGHT_FD_READ),
        EVENTTYPE_FD_WRITE => fd_event(caller, subscription, EVENTTYPE_FD_WRITE, RIGHT_FD_WRITE),
        EVENTTYPE_CLOCK => clock_readiness(caller, subscription),
        _ => Ok(PollReadiness::Unsupported(ERRNO_NOSYS)),
    }
}

fn clock_readiness(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> wasmtime::Result<PollReadiness> {
    Ok(match immediate_clock_event(caller, subscription) {
        Ok(Some(event)) => PollReadiness::Ready(event),
        Ok(None) => PollReadiness::WaitingClock,
        Err(errno) => PollReadiness::Unsupported(errno),
    })
}

fn fd_event(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
    event_type: u8,
    required_right: u64,
) -> wasmtime::Result<PollReadiness> {
    let Some(host) = caller.data().wasi_host() else {
        return Ok(PollReadiness::Unsupported(ERRNO_NOSYS));
    };
    let fd = layout::subscription_fd(subscription);
    let mut host = host
        .lock()
        .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
    let errno = match host.fd_fdstat_get(fd) {
        Ok(stat) => readiness_errno(stat, required_right),
        Err(errno) => Some(errno),
    };
    if let Some(errno) = errno {
        return Ok(PollReadiness::Ready(event_with_errno(
            subscription,
            event_type,
            Some(errno),
        )));
    }
    Ok(fd_readiness_event(
        host.as_mut(),
        fd,
        subscription,
        event_type,
    ))
}

fn fd_readiness_event(
    host: &mut dyn super::QuickJsWasiHost,
    fd: u32,
    subscription: &layout::Subscription,
    event_type: u8,
) -> PollReadiness {
    let ready = if event_type == EVENTTYPE_FD_READ {
        host.fd_read_ready(fd)
    } else {
        host.fd_write_ready(fd)
    };
    fd_ready_event(subscription, event_type, ready)
}

fn fd_ready_event(
    subscription: &layout::Subscription,
    event_type: u8,
    ready: Result<bool, QuickJsWasiErrno>,
) -> PollReadiness {
    match ready {
        Ok(true) => PollReadiness::Ready(event_with_errno(subscription, event_type, None)),
        Ok(false) => PollReadiness::PendingFd,
        Err(errno) => PollReadiness::Ready(event_with_errno(subscription, event_type, Some(errno))),
    }
}

fn readiness_errno(stat: QuickJsWasiFdStat, required_right: u64) -> Option<QuickJsWasiErrno> {
    if stat.rights_base() & required_right == 0 {
        Some(QuickJsWasiErrno::Notcapable)
    } else {
        None
    }
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

fn clock_event(
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

fn event_with_errno(
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
