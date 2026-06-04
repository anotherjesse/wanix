use super::super::HostState;
use super::super::guest_memory::{guest_offset_at, guest_range};
use wasmtime::{Caller, Memory};

pub(super) const WASI_U32_SIZE: usize = 4;
const WASI_IOV_SIZE: usize = 2 * WASI_U32_SIZE;
const WASI_IOV_LEN_OFFSET: usize = WASI_U32_SIZE;

#[derive(Debug)]
pub(super) struct GuestIov {
    pub(super) ptr: usize,
    pub(super) len: usize,
    len_u32: u32,
}

pub(super) fn preflight_fd_read_iovs(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    iovs_len: usize,
) -> wasmtime::Result<()> {
    preflight_iov_table(memory, caller, iovs_ptr, iovs_len)?;
    sum_iov_lens(memory, caller, iovs_ptr, iovs_len).map(|_| ())
}

fn preflight_iov_table(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    iovs_len: usize,
) -> wasmtime::Result<()> {
    let Some(last_index) = iovs_len.checked_sub(1) else {
        return Ok(());
    };
    guest_range(
        memory,
        caller,
        guest_offset_at(iovs_ptr, last_index, WASI_IOV_SIZE)?,
        WASI_IOV_SIZE,
    )?;
    Ok(())
}

fn sum_iov_lens(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    iovs_len: usize,
) -> wasmtime::Result<u32> {
    let mut total_len = 0u32;
    for index in 0..iovs_len {
        let iov = read_valid_iov(memory, caller, iovs_ptr, index)?;
        total_len = checked_fd_read_total(total_len, iov.len_u32)?;
    }
    Ok(total_len)
}

pub(super) fn read_valid_iov(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    index: usize,
) -> wasmtime::Result<GuestIov> {
    let iov_offset = guest_offset_at(iovs_ptr, index, WASI_IOV_SIZE)?;
    let iov = read_iov_descriptor(memory, caller, iov_offset)?;
    let len_u32 = read_iov_u32(&iov, WASI_IOV_LEN_OFFSET);
    guest_iov_from_descriptor(memory, caller, &iov, len_u32)
}

fn read_iov_descriptor(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iov_offset: usize,
) -> wasmtime::Result<[u8; WASI_IOV_SIZE]> {
    let mut iov = [0u8; WASI_IOV_SIZE];
    memory.read(caller, iov_offset, &mut iov)?;
    Ok(iov)
}

fn guest_iov_from_descriptor(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iov: &[u8; WASI_IOV_SIZE],
    len_u32: u32,
) -> wasmtime::Result<GuestIov> {
    let ptr = usize::try_from(read_iov_u32(iov, 0))
        .map_err(|_| wasmtime::Error::msg("guest iov pointer does not fit host usize"))?;
    let len = usize::try_from(len_u32)
        .map_err(|_| wasmtime::Error::msg("guest iov length does not fit host usize"))?;
    guest_range(memory, caller, ptr, len)?;
    Ok(GuestIov { ptr, len, len_u32 })
}

fn read_iov_u32(iov: &[u8; WASI_IOV_SIZE], offset: usize) -> u32 {
    let mut field = [0; WASI_U32_SIZE];
    field.copy_from_slice(&iov[offset..offset + WASI_U32_SIZE]);
    u32::from_le_bytes(field)
}

pub(super) fn checked_fd_read_total(left: u32, right: u32) -> wasmtime::Result<u32> {
    left.checked_add(right)
        .ok_or_else(|| wasmtime::Error::msg("WASI byte count overflow"))
}

pub(super) fn fd_read_count(count: usize) -> wasmtime::Result<u32> {
    u32::try_from(count).map_err(|_| wasmtime::Error::msg("fd_read byte count exceeds u32"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fd_read_total_rejects_u32_overflow() {
        assert_eq!(checked_fd_read_total(0, 0).unwrap(), 0);
        assert_eq!(checked_fd_read_total(u32::MAX - 1, 1).unwrap(), u32::MAX);
        let err = checked_fd_read_total(u32::MAX, 1)
            .expect_err("fd_read byte count should reject overflow");
        assert!(err.to_string().contains("WASI byte count overflow"));
    }
}
