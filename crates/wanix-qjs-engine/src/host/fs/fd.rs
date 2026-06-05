use super::{
    ERRNO_BADF, ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, PREOPEN_ROOT_PATH,
    QuickJsWasiDirEntry, WASI_U32_SIZE, caller_memory, guest_len, guest_range, preview1_fd,
    with_wasi_host, with_wasi_host_u32, write_prestat, write_wasi_direntries,
};
use crate::guest::guest_offset;
use wasmtime::{Caller, Memory};

mod position;
mod stat;
pub(super) use position::{fd_close, fd_seek, fd_tell};
pub(super) use stat::{
    fd_fdstat_get, fd_fdstat_set_flags, fd_filestat_get, fd_filestat_set_size,
    fd_filestat_set_times,
};

pub(super) fn fd_prestat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    prestat_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_prestat_get(fd))? {
        let prestat = match result {
            Ok(prestat) => prestat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        write_prestat(&memory, &mut caller, prestat_ptr, &prestat)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(fd) {
        return Ok(ERRNO_BADF);
    }
    let memory = caller_memory(&caller)?;
    let mut prestat = [0u8; super::PRESTAT_SIZE];
    prestat[4..8].copy_from_slice(&1_u32.to_le_bytes());
    memory.write(&mut caller, guest_offset(prestat_ptr), &prestat)?;
    Ok(ERRNO_SUCCESS)
}

pub(super) fn fd_prestat_dir_name(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_prestat_get(fd))? {
        let prestat = match result {
            Ok(prestat) => prestat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        return write_prestat_dir_name(
            &mut caller,
            path_ptr,
            path_len,
            prestat.dir_name().as_bytes(),
        );
    }

    write_virtual_prestat_dir_name(&mut caller, fd, path_ptr, path_len)
}

fn write_virtual_prestat_dir_name(
    caller: &mut Caller<'_, HostState>,
    fd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if !caller.data().is_virtual_preopen_fd(fd) {
        return Ok(ERRNO_BADF);
    }

    write_prestat_dir_name(caller, path_ptr, path_len, PREOPEN_ROOT_PATH)
}

fn write_prestat_dir_name(
    caller: &mut Caller<'_, HostState>,
    path_ptr: i32,
    path_len: i32,
    path: &[u8],
) -> wasmtime::Result<i32> {
    let len = guest_len(path_len)?;
    if len < path.len() {
        return Ok(ERRNO_INVAL);
    }
    let memory = caller_memory(caller)?;
    memory.write(caller, guest_offset(path_ptr), path)?;
    Ok(ERRNO_SUCCESS)
}

pub(super) fn fd_readdir(
    caller: Caller<'_, HostState>,
    fd: i32,
    buf_ptr: i32,
    buf_len: i32,
    cookie: i64,
    bufused_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        return fd_readdir_with_wasi_host(
            caller,
            ReaddirRequest {
                fd,
                buf_ptr,
                buf_len,
                cookie,
                bufused_ptr,
            },
        );
    }

    if caller.data().is_virtual_preopen_fd(fd) {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

struct ReaddirRequest {
    fd: i32,
    buf_ptr: i32,
    buf_len: i32,
    cookie: i64,
    bufused_ptr: i32,
}

fn fd_readdir_with_wasi_host(
    mut caller: Caller<'_, HostState>,
    request: ReaddirRequest,
) -> wasmtime::Result<i32> {
    let fd = match preview1_fd(request.fd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(&caller)?;
    let buffer = prepare_readdir_guest_buffer(
        &memory,
        &caller,
        request.buf_ptr,
        request.buf_len,
        request.bufused_ptr,
    )?;

    let entries = match read_wasi_direntries(&caller, fd)? {
        Ok(entries) => entries,
        Err(errno) => return Ok(errno),
    };
    write_readdir_entries(&memory, &mut caller, buffer, request.cookie, &entries)?;
    Ok(ERRNO_SUCCESS)
}

struct ReaddirGuestBuffer {
    start: usize,
    len: usize,
    used_ptr: i32,
}

fn prepare_readdir_guest_buffer(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    buf_ptr: i32,
    buf_len: i32,
    bufused_ptr: i32,
) -> wasmtime::Result<ReaddirGuestBuffer> {
    let len = guest_len(buf_len)?;
    let range = guest_range(memory, caller, guest_offset(buf_ptr), len)?;
    guest_range(memory, caller, guest_offset(bufused_ptr), WASI_U32_SIZE)?;
    Ok(ReaddirGuestBuffer {
        start: range.start,
        len,
        used_ptr: bufused_ptr,
    })
}

fn read_wasi_direntries(
    caller: &Caller<'_, HostState>,
    fd: u32,
) -> wasmtime::Result<Result<Vec<QuickJsWasiDirEntry>, i32>> {
    let Some(result) = with_wasi_host_u32(caller, |host| host.fd_readdir(fd))? else {
        return Ok(Err(ERRNO_BADF));
    };
    Ok(result.map_err(|errno| errno.preview1_result()))
}

fn write_readdir_entries(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    buffer: ReaddirGuestBuffer,
    cookie: i64,
    entries: &[QuickJsWasiDirEntry],
) -> wasmtime::Result<()> {
    let used = write_wasi_direntries(
        memory,
        caller,
        buffer.start,
        buffer.len,
        cookie.cast_unsigned(),
        entries,
    )?;
    let used = u32::try_from(used)
        .map_err(|_| wasmtime::Error::msg("fd_readdir byte count exceeds u32"))?;
    memory.write(caller, guest_offset(buffer.used_ptr), &used.to_le_bytes())?;
    Ok(())
}
