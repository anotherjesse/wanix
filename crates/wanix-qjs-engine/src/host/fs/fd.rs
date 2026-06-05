use super::{
    ERRNO_BADF, ERRNO_INVAL, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, PREOPEN_ROOT_PATH,
    WASI_U32_SIZE, caller_memory, guest_len, guest_range, preview1_fd, with_wasi_host,
    with_wasi_host_u32, write_prestat, write_wasi_direntries,
};
use crate::guest::guest_offset;
use wasmtime::Caller;

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
        let path_len = guest_len(path_len)?;
        let path = prestat.dir_name().as_bytes();
        if path_len < path.len() {
            return Ok(ERRNO_INVAL);
        }
        let memory = caller_memory(&caller)?;
        memory.write(&mut caller, guest_offset(path_ptr), path)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(fd) {
        return Ok(ERRNO_BADF);
    }
    let path_len = guest_len(path_len)?;
    if path_len < PREOPEN_ROOT_PATH.len() {
        return Ok(ERRNO_INVAL);
    }
    let memory = caller_memory(&caller)?;
    memory.write(&mut caller, guest_offset(path_ptr), PREOPEN_ROOT_PATH)?;
    Ok(ERRNO_SUCCESS)
}

pub(super) fn fd_readdir(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    buf_ptr: i32,
    buf_len: i32,
    cookie: i64,
    bufused_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let fd = match preview1_fd(fd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let buf_len = guest_len(buf_len)?;
        let memory = caller_memory(&caller)?;
        let buf_range = guest_range(&memory, &caller, guest_offset(buf_ptr), buf_len)?;
        guest_range(&memory, &caller, guest_offset(bufused_ptr), WASI_U32_SIZE)?;

        let Some(result) = with_wasi_host_u32(&caller, |host| host.fd_readdir(fd))? else {
            return Ok(ERRNO_BADF);
        };
        let entries = match result {
            Ok(entries) => entries,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let used = write_wasi_direntries(
            &memory,
            &mut caller,
            buf_range.start,
            buf_len,
            cookie.cast_unsigned(),
            &entries,
        )?;
        let used = u32::try_from(used)
            .map_err(|_| wasmtime::Error::msg("fd_readdir byte count exceeds u32"))?;
        memory.write(&mut caller, guest_offset(bufused_ptr), &used.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    if caller.data().is_virtual_preopen_fd(fd) {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}
