use std::thread;
use std::time::Duration;

use super::guest_memory::{guest_len, guest_offset_at, guest_range};
use super::{ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, caller_memory};
use super::{QuickJsWasiErrno, QuickJsWasiFdStat};
use crate::guest::guest_offset;
use wasmtime::{Caller, Linker};

mod layout;

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
    let nsubscriptions = guest_len(nsubscriptions)?;
    if nsubscriptions == 0 {
        return Ok(ERRNO_INVAL);
    }
    let memory = caller_memory(&caller)?;
    guest_range(
        &memory,
        &caller,
        guest_offset(nevents_ptr),
        layout::WASI_U32_SIZE,
    )?;
    guest_range(
        &memory,
        &caller,
        guest_offset(in_ptr),
        nsubscriptions
            .checked_mul(layout::SUBSCRIPTION_SIZE)
            .ok_or_else(|| wasmtime::Error::msg("poll_oneoff subscription size overflow"))?,
    )?;
    guest_range(
        &memory,
        &caller,
        guest_offset(out_ptr),
        nsubscriptions
            .checked_mul(layout::EVENT_SIZE)
            .ok_or_else(|| wasmtime::Error::msg("poll_oneoff event size overflow"))?,
    )?;

    let mut subscriptions = Vec::new();
    subscriptions
        .try_reserve_exact(nsubscriptions)
        .map_err(|_| wasmtime::Error::msg("poll_oneoff subscription allocation failed"))?;
    for index in 0..nsubscriptions {
        let mut subscription = [0; layout::SUBSCRIPTION_SIZE];
        memory.read(
            &caller,
            guest_offset_at(in_ptr, index, layout::SUBSCRIPTION_SIZE)?,
            &mut subscription,
        )?;
        subscriptions.push(subscription);
    }

    let mut ready_events = Vec::new();
    let mut pending_fd = false;
    for subscription in &subscriptions {
        match layout::subscription_tag(subscription) {
            EVENTTYPE_FD_READ | EVENTTYPE_FD_WRITE => {
                let event = match fd_event(&caller, subscription)? {
                    Ok(event) => event,
                    Err(errno) => return Ok(errno),
                };
                if let Some(event) = event {
                    ready_events.push(event);
                } else {
                    pending_fd = true;
                }
            }
            EVENTTYPE_CLOCK => {
                let event = match immediate_clock_event(&caller, subscription) {
                    Ok(event) => event,
                    Err(errno) => return Ok(errno),
                };
                if let Some(event) = event {
                    ready_events.push(event);
                }
            }
            _ => return Ok(ERRNO_NOSYS),
        }
    }
    if !ready_events.is_empty() {
        layout::write_events(&memory, &mut caller, out_ptr, &ready_events)?;
        layout::write_nevents(&memory, &mut caller, nevents_ptr, ready_events.len())?;
        return Ok(ERRNO_SUCCESS);
    }

    if pending_fd {
        layout::write_nevents(&memory, &mut caller, nevents_ptr, 0)?;
        return Ok(ERRNO_SUCCESS);
    }

    if subscriptions.len() != 1 {
        return Ok(ERRNO_NOSYS);
    }

    guest_range(
        &memory,
        &caller,
        guest_offset_at(in_ptr, 0, layout::SUBSCRIPTION_SIZE)?,
        layout::SUBSCRIPTION_SIZE,
    )?;
    guest_range(
        &memory,
        &caller,
        guest_offset_at(out_ptr, 0, layout::EVENT_SIZE)?,
        layout::EVENT_SIZE,
    )?;

    let event = match clock_event(&caller, &subscriptions[0]) {
        Ok(event) => event,
        Err(errno) => return Ok(errno),
    };

    memory.write(&mut caller, guest_offset(out_ptr), &event)?;
    layout::write_nevents(&memory, &mut caller, nevents_ptr, 1)?;
    Ok(ERRNO_SUCCESS)
}

fn fd_event(
    caller: &Caller<'_, HostState>,
    subscription: &layout::Subscription,
) -> wasmtime::Result<Result<Option<layout::Event>, i32>> {
    let event_type = layout::subscription_tag(subscription);
    let required_right = match event_type {
        EVENTTYPE_FD_READ => RIGHT_FD_READ,
        EVENTTYPE_FD_WRITE => RIGHT_FD_WRITE,
        _ => return Ok(Err(ERRNO_NOSYS)),
    };
    let Some(host) = caller.data().wasi_host() else {
        return Ok(Err(ERRNO_NOSYS));
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
        return Ok(Ok(Some(event_with_errno(
            subscription,
            event_type,
            Some(errno),
        ))));
    }
    let ready = match event_type {
        EVENTTYPE_FD_READ => host.fd_read_ready(fd),
        EVENTTYPE_FD_WRITE => host.fd_write_ready(fd),
        _ => unreachable!("fd event type checked above"),
    };
    match ready {
        Ok(true) => Ok(Ok(Some(event_with_errno(subscription, event_type, None)))),
        Ok(false) => Ok(Ok(None)),
        Err(errno) => Ok(Ok(Some(event_with_errno(
            subscription,
            event_type,
            Some(errno),
        )))),
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
    if layout::subscription_tag(subscription) != EVENTTYPE_CLOCK {
        return Err(ERRNO_NOSYS);
    }

    let clock_id = layout::subscription_clock_id(subscription);
    if clock_id != CLOCKID_REALTIME && clock_id != CLOCKID_MONOTONIC {
        return Err(ERRNO_NOSYS);
    }

    let timeout_ns = layout::subscription_clock_timeout(subscription);
    let flags = layout::subscription_clock_flags(subscription);
    if flags & !SUBCLOCKFLAGS_ABSTIME != 0 {
        return Err(ERRNO_INVAL);
    }

    let sleep_ns = if flags & SUBCLOCKFLAGS_ABSTIME != 0 {
        timeout_ns.saturating_sub(caller.data().config().clock_time_ns())
    } else {
        timeout_ns
    };
    Ok(sleep_ns)
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
