use super::HostState;
use std::io::Write;
use wasmtime::{Caller, Memory};

const HOST_TRANSFER_CHUNK_LEN: usize = 16 * 1024;

pub(super) fn guest_offset_at(base: i32, index: usize, stride: usize) -> wasmtime::Result<usize> {
    crate::guest::guest_offset_at(base, index, stride)
        .ok_or_else(|| wasmtime::Error::msg("guest pointer offset overflow"))
}

fn checked_guest_range(
    memory_len: usize,
    ptr: usize,
    len: usize,
) -> wasmtime::Result<std::ops::Range<usize>> {
    let end = ptr
        .checked_add(len)
        .ok_or_else(|| wasmtime::Error::msg("guest memory range overflow"))?;
    if end > memory_len {
        return Err(wasmtime::Error::msg("guest memory range is outside memory"));
    }
    Ok(ptr..end)
}

pub(super) fn guest_range(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: usize,
    len: usize,
) -> wasmtime::Result<std::ops::Range<usize>> {
    checked_guest_range(memory.data_size(caller), ptr, len)
}

pub(super) fn guest_len(value: i32) -> wasmtime::Result<usize> {
    crate::guest::guest_len(value).ok_or_else(|| wasmtime::Error::msg("negative guest length"))
}

pub(super) fn write_guest_buffer(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    mut writer: impl Write,
) -> wasmtime::Result<()> {
    visit_guest_buffer_chunks(memory, caller, ptr, len, |_state, chunk| {
        writer.write_all(chunk)?;
        Ok(())
    })
}

pub(super) fn capture_guest_buffer(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    append: impl FnMut(&mut HostState, &[u8]) -> wasmtime::Result<()>,
) -> wasmtime::Result<()> {
    visit_guest_buffer_chunks(memory, caller, ptr, len, append)
}

fn visit_guest_buffer_chunks(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    mut visit: impl FnMut(&mut HostState, &[u8]) -> wasmtime::Result<()>,
) -> wasmtime::Result<()> {
    let range = guest_range(memory, &*caller, ptr, len)?;
    let mut chunk = [0u8; HOST_TRANSFER_CHUNK_LEN];
    let mut offset = range.start;
    while offset < range.end {
        let chunk_len = (range.end - offset).min(chunk.len());
        memory.read(&*caller, offset, &mut chunk[..chunk_len])?;
        visit(caller.data_mut(), &chunk[..chunk_len])?;
        offset += chunk_len;
    }
    Ok(())
}

pub(super) fn fill_guest_buffer(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    byte: u8,
) -> wasmtime::Result<()> {
    let range = checked_guest_range(memory.data_size(&*caller), ptr, len)?;
    let chunk = [byte; HOST_TRANSFER_CHUNK_LEN];
    let mut offset = range.start;
    while offset < range.end {
        let chunk_len = (range.end - offset).min(chunk.len());
        memory.write(&mut *caller, offset, &chunk[..chunk_len])?;
        offset += chunk_len;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_guest_offsets_check_overflow() {
        assert_eq!(guest_offset_at(-8, 1, 4).unwrap(), u32::MAX as usize - 3);
        assert!(guest_offset_at(-4, 1, 8).is_err());
        assert!(guest_offset_at(0, usize::MAX, 8).is_err());
    }

    #[test]
    fn guest_lengths_reject_negative_values() {
        assert!(guest_len(-1).is_err());
        assert_eq!(guest_len(i32::MAX).unwrap(), i32::MAX as usize);
    }

    #[test]
    fn guest_ranges_check_bounds_and_allow_empty_ranges() {
        assert_eq!(checked_guest_range(16, 4, 0).unwrap(), 4..4);
        assert_eq!(checked_guest_range(16, 4, 12).unwrap(), 4..16);
        assert!(checked_guest_range(16, 4, 13).is_err());
        assert!(checked_guest_range(usize::MAX, usize::MAX, 1).is_err());
    }
}
