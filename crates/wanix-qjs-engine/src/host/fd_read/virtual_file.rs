use super::super::{ERRNO_BADF, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, HostState};
use super::iov::{GuestIov, checked_fd_read_total, fd_read_count, read_valid_iov};
use super::{ReadRequest, read_request, write_nread};
use std::sync::Arc;
use wasmtime::{Caller, Memory};

const RIGHT_FD_READ: u64 = 1 << 1;

struct VirtualReadTarget {
    bytes: Arc<[u8]>,
    offset: u64,
    rights_base: u64,
}

pub(super) fn fd_read_virtual_file(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    iovs_ptr: i32,
    iovs_len: i32,
    nread_ptr: i32,
) -> wasmtime::Result<i32> {
    let read_result = read_from_virtual_file(&mut caller, fd, iovs_ptr, iovs_len, nread_ptr)?;
    finish_virtual_fd_read(&mut caller, fd, nread_ptr, read_result)
}

fn read_from_virtual_file(
    caller: &mut Caller<'_, HostState>,
    fd: i32,
    iovs_ptr: i32,
    iovs_len: i32,
    nread_ptr: i32,
) -> wasmtime::Result<Result<(ReadRequest, VirtualReadTarget, u32), i32>> {
    let target = match readable_virtual_target(caller, fd) {
        Ok(target) => target,
        Err(errno) => return Ok(Err(errno)),
    };
    let request = read_request(caller, iovs_ptr, iovs_len, nread_ptr)?;
    let total_read = read_virtual_iovs(&request, caller, iovs_ptr, &target)?;
    Ok(Ok((request, target, total_read)))
}

fn finish_virtual_fd_read(
    caller: &mut Caller<'_, HostState>,
    fd: i32,
    nread_ptr: i32,
    read_result: Result<(ReadRequest, VirtualReadTarget, u32), i32>,
) -> wasmtime::Result<i32> {
    let (request, target, total_read) = match read_result {
        Ok(result) => result,
        Err(errno) => return Ok(errno),
    };
    write_nread(&request.memory, caller, nread_ptr, total_read)?;
    update_virtual_offset(caller, fd, target.offset, total_read);
    Ok(ERRNO_SUCCESS)
}

fn virtual_read_target(caller: &Caller<'_, HostState>, fd: i32) -> Option<VirtualReadTarget> {
    caller
        .data()
        .virtual_file(fd)
        .map(|file| VirtualReadTarget {
            bytes: Arc::clone(&file.bytes),
            offset: file.offset,
            rights_base: file.rights_base,
        })
}

fn readable_virtual_target(
    caller: &Caller<'_, HostState>,
    fd: i32,
) -> Result<VirtualReadTarget, i32> {
    let Some(target) = virtual_read_target(caller, fd) else {
        return Err(ERRNO_BADF);
    };
    if target.rights_base & RIGHT_FD_READ == 0 {
        return Err(ERRNO_NOTCAPABLE);
    }
    Ok(target)
}

fn read_virtual_iovs(
    request: &ReadRequest,
    caller: &mut Caller<'_, HostState>,
    iovs_ptr: i32,
    target: &VirtualReadTarget,
) -> wasmtime::Result<u32> {
    let mut total_read = 0u32;
    let mut file_offset = usize::try_from(target.offset)
        .unwrap_or(usize::MAX)
        .min(target.bytes.len());
    for index in 0..request.iovs_len {
        let iov = read_valid_iov(&request.memory, &*caller, iovs_ptr, index)?;
        let chunk_len = write_virtual_iov(
            &request.memory,
            caller,
            target.bytes.as_ref(),
            file_offset,
            iov,
        )?;
        file_offset += chunk_len;
        total_read = checked_fd_read_total(total_read, fd_read_count(chunk_len)?)?;
    }
    Ok(total_read)
}

fn update_virtual_offset(
    caller: &mut Caller<'_, HostState>,
    fd: i32,
    offset: u64,
    total_read: u32,
) {
    if let Some(file) = caller.data_mut().virtual_file_mut(fd) {
        file.offset = offset.saturating_add(u64::from(total_read));
    }
}

fn write_virtual_iov(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    bytes: &[u8],
    file_offset: usize,
    iov: GuestIov,
) -> wasmtime::Result<usize> {
    if file_offset == bytes.len() || iov.len == 0 {
        return Ok(0);
    }
    let chunk_len = iov.len.min(bytes.len() - file_offset);
    memory.write(
        caller,
        iov.ptr,
        &bytes[file_offset..file_offset + chunk_len],
    )?;
    Ok(chunk_len)
}
