use std::thread;
use std::time::Duration;

use super::guest_memory::{guest_len, guest_offset_at, guest_range};
use super::{ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, caller_memory};
use super::{QuickJsWasiErrno, QuickJsWasiFdStat};
use crate::guest::guest_offset;
use wasmtime::{Caller, Linker, Memory};

const WASI_U32_SIZE: usize = 4;
const SUBSCRIPTION_SIZE: usize = 48;
const EVENT_SIZE: usize = 32;

const SUBSCRIPTION_USERDATA_OFFSET: usize = 0;
const SUBSCRIPTION_TAG_OFFSET: usize = 8;
const SUBSCRIPTION_CLOCK_ID_OFFSET: usize = 16;
const SUBSCRIPTION_CLOCK_TIMEOUT_OFFSET: usize = 24;
const SUBSCRIPTION_CLOCK_FLAGS_OFFSET: usize = 40;
const SUBSCRIPTION_FD_OFFSET: usize = 16;

const EVENT_USERDATA_OFFSET: usize = 0;
const EVENT_ERROR_OFFSET: usize = 8;
const EVENT_TYPE_OFFSET: usize = 10;

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
    guest_range(&memory, &caller, guest_offset(nevents_ptr), WASI_U32_SIZE)?;
    guest_range(
        &memory,
        &caller,
        guest_offset(in_ptr),
        nsubscriptions
            .checked_mul(SUBSCRIPTION_SIZE)
            .ok_or_else(|| wasmtime::Error::msg("poll_oneoff subscription size overflow"))?,
    )?;
    guest_range(
        &memory,
        &caller,
        guest_offset(out_ptr),
        nsubscriptions
            .checked_mul(EVENT_SIZE)
            .ok_or_else(|| wasmtime::Error::msg("poll_oneoff event size overflow"))?,
    )?;

    let mut subscriptions = Vec::new();
    subscriptions
        .try_reserve_exact(nsubscriptions)
        .map_err(|_| wasmtime::Error::msg("poll_oneoff subscription allocation failed"))?;
    for index in 0..nsubscriptions {
        let mut subscription = [0; SUBSCRIPTION_SIZE];
        memory.read(
            &caller,
            guest_offset_at(in_ptr, index, SUBSCRIPTION_SIZE)?,
            &mut subscription,
        )?;
        subscriptions.push(subscription);
    }

    let mut ready_events = Vec::new();
    let mut pending_fd = false;
    for subscription in &subscriptions {
        match subscription[SUBSCRIPTION_TAG_OFFSET] {
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
        write_events(&memory, &mut caller, out_ptr, &ready_events)?;
        write_nevents(&memory, &mut caller, nevents_ptr, ready_events.len())?;
        return Ok(ERRNO_SUCCESS);
    }

    if pending_fd {
        write_nevents(&memory, &mut caller, nevents_ptr, 0)?;
        return Ok(ERRNO_SUCCESS);
    }

    if subscriptions.len() != 1 {
        return Ok(ERRNO_NOSYS);
    }

    guest_range(
        &memory,
        &caller,
        guest_offset_at(in_ptr, 0, SUBSCRIPTION_SIZE)?,
        SUBSCRIPTION_SIZE,
    )?;
    guest_range(
        &memory,
        &caller,
        guest_offset_at(out_ptr, 0, EVENT_SIZE)?,
        EVENT_SIZE,
    )?;

    let event = match clock_event(&caller, &subscriptions[0]) {
        Ok(event) => event,
        Err(errno) => return Ok(errno),
    };

    memory.write(&mut caller, guest_offset(out_ptr), &event)?;
    write_nevents(&memory, &mut caller, nevents_ptr, 1)?;
    Ok(ERRNO_SUCCESS)
}

fn fd_event(
    caller: &Caller<'_, HostState>,
    subscription: &[u8; SUBSCRIPTION_SIZE],
) -> wasmtime::Result<Result<Option<[u8; EVENT_SIZE]>, i32>> {
    let event_type = subscription[SUBSCRIPTION_TAG_OFFSET];
    let required_right = match event_type {
        EVENTTYPE_FD_READ => RIGHT_FD_READ,
        EVENTTYPE_FD_WRITE => RIGHT_FD_WRITE,
        _ => return Ok(Err(ERRNO_NOSYS)),
    };
    let Some(host) = caller.data().wasi_host() else {
        return Ok(Err(ERRNO_NOSYS));
    };
    let fd = read_u32(subscription, SUBSCRIPTION_FD_OFFSET);
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
    subscription: &[u8; SUBSCRIPTION_SIZE],
) -> Result<Option<[u8; EVENT_SIZE]>, i32> {
    let sleep_ns = clock_sleep_ns(caller, subscription)?;
    if sleep_ns == 0 {
        Ok(Some(event_with_errno(subscription, EVENTTYPE_CLOCK, None)))
    } else {
        Ok(None)
    }
}

fn clock_event(
    caller: &Caller<'_, HostState>,
    subscription: &[u8; SUBSCRIPTION_SIZE],
) -> Result<[u8; EVENT_SIZE], i32> {
    let sleep_ns = clock_sleep_ns(caller, subscription)?;
    if sleep_ns != 0 {
        thread::sleep(Duration::from_nanos(sleep_ns));
    }
    Ok(event_with_errno(subscription, EVENTTYPE_CLOCK, None))
}

fn clock_sleep_ns(
    caller: &Caller<'_, HostState>,
    subscription: &[u8; SUBSCRIPTION_SIZE],
) -> Result<u64, i32> {
    if subscription[SUBSCRIPTION_TAG_OFFSET] != EVENTTYPE_CLOCK {
        return Err(ERRNO_NOSYS);
    }

    let clock_id = read_u32(subscription, SUBSCRIPTION_CLOCK_ID_OFFSET);
    if clock_id != CLOCKID_REALTIME && clock_id != CLOCKID_MONOTONIC {
        return Err(ERRNO_NOSYS);
    }

    let timeout_ns = read_u64(subscription, SUBSCRIPTION_CLOCK_TIMEOUT_OFFSET);
    let flags = read_u16(subscription, SUBSCRIPTION_CLOCK_FLAGS_OFFSET);
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
    subscription: &[u8; SUBSCRIPTION_SIZE],
    event_type: u8,
    errno: Option<QuickJsWasiErrno>,
) -> [u8; EVENT_SIZE] {
    let mut event = [0; EVENT_SIZE];
    event[EVENT_USERDATA_OFFSET..EVENT_USERDATA_OFFSET + 8].copy_from_slice(
        &subscription[SUBSCRIPTION_USERDATA_OFFSET..SUBSCRIPTION_USERDATA_OFFSET + 8],
    );
    let errno = errno.map_or(0, |errno| errno.preview1_result() as u16);
    event[EVENT_ERROR_OFFSET..EVENT_ERROR_OFFSET + 2].copy_from_slice(&errno.to_le_bytes());
    event[EVENT_TYPE_OFFSET] = event_type;
    event
}

fn write_events(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    out_ptr: i32,
    events: &[[u8; EVENT_SIZE]],
) -> wasmtime::Result<()> {
    for (index, event) in events.iter().enumerate() {
        memory.write(
            &mut *caller,
            guest_offset_at(out_ptr, index, EVENT_SIZE)?,
            event,
        )?;
    }
    Ok(())
}

fn write_nevents(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    nevents_ptr: i32,
    nevents: usize,
) -> wasmtime::Result<()> {
    let nevents = u32::try_from(nevents)
        .map_err(|_| wasmtime::Error::msg("poll_oneoff event count exceeds u32"))?;
    memory.write(caller, guest_offset(nevents_ptr), &nevents.to_le_bytes())?;
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
