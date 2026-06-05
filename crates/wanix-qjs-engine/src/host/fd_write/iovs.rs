use super::super::{HostState, guest_memory::guest_range};
use super::WASI_U32_SIZE;
use crate::host::guest_memory::guest_offset_at;
use wasmtime::{Caller, Memory};

const WASI_IOV_SIZE: usize = 2 * WASI_U32_SIZE;
const WASI_IOV_LEN_OFFSET: usize = WASI_U32_SIZE;

#[derive(Debug)]
pub(super) struct GuestIov {
    pub(super) ptr: usize,
    pub(super) len: usize,
    len_u32: u32,
}

pub(super) fn preflight_fd_write_iovs(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    iovs_len: usize,
) -> wasmtime::Result<u32> {
    if iovs_len > 0 {
        guest_range(
            memory,
            caller,
            guest_offset_at(iovs_ptr, iovs_len - 1, WASI_IOV_SIZE)?,
            WASI_IOV_SIZE,
        )?;
    }

    let mut total_written = 0u32;
    for index in 0..iovs_len {
        let iov = read_valid_fd_iov(memory, caller, iovs_ptr, index)?;
        total_written = checked_fd_write_total(total_written, iov.len_u32)?;
    }
    Ok(total_written)
}

pub(super) fn read_valid_fd_iov(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    index: usize,
) -> wasmtime::Result<GuestIov> {
    let iov_offset = guest_offset_at(iovs_ptr, index, WASI_IOV_SIZE)?;
    let mut iov = [0u8; WASI_IOV_SIZE];
    memory.read(caller, iov_offset, &mut iov)?;

    let ptr = read_iov_u32(&iov, 0);
    let len_u32 = read_iov_u32(&iov, WASI_IOV_LEN_OFFSET);
    let ptr = usize::try_from(ptr)
        .map_err(|_| wasmtime::Error::msg("guest iov pointer does not fit host usize"))?;
    let len = usize::try_from(len_u32)
        .map_err(|_| wasmtime::Error::msg("guest iov length does not fit host usize"))?;

    guest_range(memory, caller, ptr, len)?;
    Ok(GuestIov { ptr, len, len_u32 })
}

fn read_iov_u32(iov: &[u8; WASI_IOV_SIZE], offset: usize) -> u32 {
    let mut field = [0u8; WASI_U32_SIZE];
    field.copy_from_slice(&iov[offset..offset + WASI_U32_SIZE]);
    u32::from_le_bytes(field)
}

pub(super) fn checked_fd_write_total(total: u32, len: u32) -> wasmtime::Result<u32> {
    total
        .checked_add(len)
        .ok_or_else(|| wasmtime::Error::msg("fd_write byte count overflow"))
}
