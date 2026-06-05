use self::host_iovs::read_host_iovs;
use self::iov::{WASI_U32_SIZE, preflight_fd_read_iovs};
use self::virtual_file::fd_read_virtual_file;
use super::guest_memory::{guest_len, guest_range};
use super::{ERRNO_BADF, ERRNO_SUCCESS, HostState, QuickJsWasiErrno, caller_memory};
use crate::guest::guest_offset;
use wasmtime::{Caller, Linker, Memory};

mod host_iovs;
mod iov;
mod virtual_file;

pub(super) struct ReadRequest {
    pub(super) memory: Memory,
    pub(super) iovs_len: usize,
}

pub(super) fn define_import(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_read",
        |caller: Caller<'_, HostState>,
         fd: i32,
         iovs_ptr: i32,
         iovs_len: i32,
         nread_ptr: i32|
         -> wasmtime::Result<i32> {
            if caller.data().wasi_host().is_some() {
                fd_read_with_wasi_host(caller, fd, iovs_ptr, iovs_len, nread_ptr)
            } else {
                fd_read_virtual_file(caller, fd, iovs_ptr, iovs_len, nread_ptr)
            }
        },
    )?;
    Ok(())
}

fn fd_read_with_wasi_host(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    iovs_ptr: i32,
    iovs_len: i32,
    nread_ptr: i32,
) -> wasmtime::Result<i32> {
    let fd = match preview1_fd(fd) {
        Ok(fd) => fd,
        Err(_) => return Ok(ERRNO_BADF),
    };
    let read_result = read_from_wasi_host(&mut caller, fd, iovs_ptr, iovs_len, nread_ptr)?;
    finish_host_fd_read(&mut caller, nread_ptr, read_result)
}

fn finish_host_fd_read(
    caller: &mut Caller<'_, HostState>,
    nread_ptr: i32,
    read_result: Result<(ReadRequest, u32), i32>,
) -> wasmtime::Result<i32> {
    match read_result {
        Ok((request, total_read)) => {
            write_nread(&request.memory, caller, nread_ptr, total_read)?;
            Ok(ERRNO_SUCCESS)
        }
        Err(errno) => Ok(errno),
    }
}

fn read_from_wasi_host(
    caller: &mut Caller<'_, HostState>,
    fd: u32,
    iovs_ptr: i32,
    iovs_len: i32,
    nread_ptr: i32,
) -> wasmtime::Result<Result<(ReadRequest, u32), i32>> {
    let request = read_request(caller, iovs_ptr, iovs_len, nread_ptr)?;
    let Some(host) = caller.data().wasi_host() else {
        return Ok(Err(ERRNO_BADF));
    };
    let mut host = host
        .lock()
        .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
    let total_read = host_read_result(read_host_iovs(&mut **host, fd, &request, caller, iovs_ptr)?);
    let total_read = match total_read {
        Ok(total_read) => total_read,
        Err(errno) => return Ok(Err(errno)),
    };
    Ok(Ok((request, total_read)))
}

fn host_read_result(result: Result<u32, QuickJsWasiErrno>) -> Result<u32, i32> {
    result.map_err(|errno| errno.preview1_result())
}

fn preview1_fd(fd: i32) -> Result<u32, i32> {
    u32::try_from(fd).map_err(|_| ERRNO_BADF)
}

pub(super) fn read_request(
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    iovs_len: i32,
    nread_ptr: i32,
) -> wasmtime::Result<ReadRequest> {
    let memory = caller_memory(caller)?;
    let iovs_len = guest_len(iovs_len)?;
    guest_range(&memory, caller, guest_offset(nread_ptr), WASI_U32_SIZE)?;
    preflight_fd_read_iovs(&memory, caller, iovs_ptr, iovs_len)?;
    Ok(ReadRequest { memory, iovs_len })
}

pub(super) fn write_nread(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    nread_ptr: i32,
    total_read: u32,
) -> wasmtime::Result<()> {
    Ok(memory.write(caller, guest_offset(nread_ptr), &total_read.to_le_bytes())?)
}
