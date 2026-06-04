use crate::guest::guest_offset;
use crate::host::HostState;
use crate::host::guest_memory::guest_offset_at;
use wasmtime::{Caller, Memory};

pub(super) const WASI_U32_SIZE: usize = 4;
pub(super) const SUBSCRIPTION_SIZE: usize = 48;
pub(super) const EVENT_SIZE: usize = 32;

const SUBSCRIPTION_USERDATA_OFFSET: usize = 0;
const SUBSCRIPTION_TAG_OFFSET: usize = 8;
const SUBSCRIPTION_CLOCK_ID_OFFSET: usize = 16;
const SUBSCRIPTION_CLOCK_TIMEOUT_OFFSET: usize = 24;
const SUBSCRIPTION_CLOCK_FLAGS_OFFSET: usize = 40;
const SUBSCRIPTION_FD_OFFSET: usize = 16;

const EVENT_USERDATA_OFFSET: usize = 0;
const EVENT_ERROR_OFFSET: usize = 8;
const EVENT_TYPE_OFFSET: usize = 10;

pub(super) type Subscription = [u8; SUBSCRIPTION_SIZE];
pub(super) type Event = [u8; EVENT_SIZE];

pub(super) fn subscription_tag(subscription: &Subscription) -> u8 {
    subscription[SUBSCRIPTION_TAG_OFFSET]
}

pub(super) fn subscription_fd(subscription: &Subscription) -> u32 {
    read_u32(subscription, SUBSCRIPTION_FD_OFFSET)
}

pub(super) fn subscription_clock_id(subscription: &Subscription) -> u32 {
    read_u32(subscription, SUBSCRIPTION_CLOCK_ID_OFFSET)
}

pub(super) fn subscription_clock_timeout(subscription: &Subscription) -> u64 {
    read_u64(subscription, SUBSCRIPTION_CLOCK_TIMEOUT_OFFSET)
}

pub(super) fn subscription_clock_flags(subscription: &Subscription) -> u16 {
    read_u16(subscription, SUBSCRIPTION_CLOCK_FLAGS_OFFSET)
}

pub(super) fn event_with_userdata(subscription: &Subscription) -> Event {
    let mut event = [0; EVENT_SIZE];
    event[EVENT_USERDATA_OFFSET..EVENT_USERDATA_OFFSET + 8].copy_from_slice(
        &subscription[SUBSCRIPTION_USERDATA_OFFSET..SUBSCRIPTION_USERDATA_OFFSET + 8],
    );
    event
}

pub(super) fn set_event_errno(event: &mut Event, errno: u16) {
    event[EVENT_ERROR_OFFSET..EVENT_ERROR_OFFSET + 2].copy_from_slice(&errno.to_le_bytes());
}

pub(super) fn set_event_type(event: &mut Event, event_type: u8) {
    event[EVENT_TYPE_OFFSET] = event_type;
}

pub(super) fn write_events(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    out_ptr: i32,
    events: &[Event],
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

pub(super) fn write_nevents(
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
