use std::thread;
use std::time::Duration;

use super::guest_memory::{guest_len, guest_offset_at, guest_range};
use super::{ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, caller_memory};
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

const EVENT_USERDATA_OFFSET: usize = 0;
const EVENT_ERROR_OFFSET: usize = 8;
const EVENT_TYPE_OFFSET: usize = 10;

const EVENTTYPE_CLOCK: u8 = 0;
const CLOCKID_REALTIME: u32 = 0;
const CLOCKID_MONOTONIC: u32 = 1;
const SUBCLOCKFLAGS_ABSTIME: u16 = 1 << 0;

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
    if nsubscriptions != 1 {
        return Ok(ERRNO_NOSYS);
    }

    let memory = caller_memory(&caller)?;
    guest_range(&memory, &caller, guest_offset(nevents_ptr), WASI_U32_SIZE)?;
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

    let mut subscription = [0; SUBSCRIPTION_SIZE];
    memory.read(&caller, guest_offset(in_ptr), &mut subscription)?;
    let event = match clock_event(&caller, &subscription) {
        Ok(event) => event,
        Err(errno) => return Ok(errno),
    };

    memory.write(&mut caller, guest_offset(out_ptr), &event)?;
    write_nevents(&memory, &mut caller, nevents_ptr, 1)?;
    Ok(ERRNO_SUCCESS)
}

fn clock_event(
    caller: &Caller<'_, HostState>,
    subscription: &[u8; SUBSCRIPTION_SIZE],
) -> Result<[u8; EVENT_SIZE], i32> {
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
    if sleep_ns != 0 {
        thread::sleep(Duration::from_nanos(sleep_ns));
    }

    let mut event = [0; EVENT_SIZE];
    event[EVENT_USERDATA_OFFSET..EVENT_USERDATA_OFFSET + 8].copy_from_slice(
        &subscription[SUBSCRIPTION_USERDATA_OFFSET..SUBSCRIPTION_USERDATA_OFFSET + 8],
    );
    event[EVENT_ERROR_OFFSET..EVENT_ERROR_OFFSET + 2].copy_from_slice(&0_u16.to_le_bytes());
    event[EVENT_TYPE_OFFSET] = EVENTTYPE_CLOCK;
    Ok(event)
}

fn write_nevents(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    nevents_ptr: i32,
    nevents: u32,
) -> wasmtime::Result<()> {
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
