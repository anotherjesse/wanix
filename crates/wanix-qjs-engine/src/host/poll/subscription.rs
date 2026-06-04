use crate::guest::guest_offset;
use crate::host::HostState;
use crate::host::guest_memory::{guest_len, guest_offset_at, guest_range};
use wasmtime::{Caller, Memory};

use super::layout;

pub(super) fn read_poll_subscriptions(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    in_ptr: i32,
    out_ptr: i32,
    nevents_ptr: i32,
    nsubscriptions: i32,
) -> wasmtime::Result<Vec<layout::Subscription>> {
    let nsubscriptions = guest_len(nsubscriptions)?;
    preflight_poll_memory(memory, caller, in_ptr, out_ptr, nevents_ptr, nsubscriptions)?;
    read_subscriptions(memory, caller, in_ptr, nsubscriptions)
}

fn read_subscriptions(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    in_ptr: i32,
    nsubscriptions: usize,
) -> wasmtime::Result<Vec<layout::Subscription>> {
    let mut subscriptions = allocate_subscriptions(nsubscriptions)?;
    for index in 0..nsubscriptions {
        subscriptions.push(read_subscription(memory, caller, in_ptr, index)?);
    }
    Ok(subscriptions)
}

fn allocate_subscriptions(nsubscriptions: usize) -> wasmtime::Result<Vec<layout::Subscription>> {
    let mut subscriptions = Vec::new();
    subscriptions
        .try_reserve_exact(nsubscriptions)
        .map_err(|_| wasmtime::Error::msg("poll_oneoff subscription allocation failed"))?;
    Ok(subscriptions)
}

fn read_subscription(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    in_ptr: i32,
    index: usize,
) -> wasmtime::Result<layout::Subscription> {
    let mut subscription = [0; layout::SUBSCRIPTION_SIZE];
    memory.read(
        caller,
        guest_offset_at(in_ptr, index, layout::SUBSCRIPTION_SIZE)?,
        &mut subscription,
    )?;
    Ok(subscription)
}

fn preflight_poll_memory(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    in_ptr: i32,
    out_ptr: i32,
    nevents_ptr: i32,
    nsubscriptions: usize,
) -> wasmtime::Result<()> {
    preflight_nevents(memory, caller, nevents_ptr)?;
    preflight_table(
        memory,
        caller,
        in_ptr,
        nsubscriptions,
        PollTable::Subscription,
    )?;
    preflight_table(memory, caller, out_ptr, nsubscriptions, PollTable::Event)?;
    Ok(())
}

fn preflight_nevents(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    nevents_ptr: i32,
) -> wasmtime::Result<()> {
    guest_range(
        memory,
        caller,
        guest_offset(nevents_ptr),
        layout::WASI_U32_SIZE,
    )?;
    Ok(())
}

fn preflight_table(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: i32,
    nsubscriptions: usize,
    table: PollTable,
) -> wasmtime::Result<()> {
    guest_range(
        memory,
        caller,
        guest_offset(ptr),
        poll_table_len(nsubscriptions, table.entry_len(), table.label())?,
    )?;
    Ok(())
}

enum PollTable {
    Subscription,
    Event,
}

impl PollTable {
    fn entry_len(&self) -> usize {
        match self {
            Self::Subscription => layout::SUBSCRIPTION_SIZE,
            Self::Event => layout::EVENT_SIZE,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::Event => "event",
        }
    }
}

fn poll_table_len(nsubscriptions: usize, entry_len: usize, label: &str) -> wasmtime::Result<usize> {
    nsubscriptions
        .checked_mul(entry_len)
        .ok_or_else(|| wasmtime::Error::msg(format!("poll_oneoff {label} size overflow")))
}
