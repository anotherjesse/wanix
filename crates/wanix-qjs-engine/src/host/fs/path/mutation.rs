use super::super::{
    ERRNO_BADF, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, HostState, QuickJsWasiErrno, caller_memory,
    checked_wasi_path_len, preview1_fd, preview1_u16_filestat_flags, read_guest_path,
    with_wasi_host_u32,
};
use super::{unsupported_lookupflags, unsupported_path_mutation};
use wasmtime::Caller;

mod rename;
mod symlink;
pub(in crate::host::fs) use rename::path_rename;
pub(in crate::host::fs) use symlink::path_symlink;

fn live_path_input(
    caller: &Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<Result<(u32, Vec<u8>), i32>> {
    let dirfd = match preview1_fd(dirfd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(Err(errno)),
    };
    let path_len = match checked_wasi_path_len(path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(Err(errno)),
    };
    let path = read_checked_live_path(caller, path_ptr, path_len)?;
    Ok(Ok((dirfd, path)))
}

fn read_checked_live_path(
    caller: &Caller<'_, HostState>,
    path_ptr: i32,
    path_len: usize,
) -> wasmtime::Result<Vec<u8>> {
    let memory = caller_memory(caller)?;
    read_guest_path(&memory, caller, path_ptr, path_len)
}

fn live_unit_result(result: Option<Result<(), QuickJsWasiErrno>>) -> i32 {
    match result {
        Some(Ok(())) => ERRNO_SUCCESS,
        Some(Err(errno)) => errno.preview1_result(),
        None => ERRNO_BADF,
    }
}

pub(in crate::host::fs) fn path_create_directory(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let (dirfd, path) = match live_path_input(&caller, dirfd, path_ptr, path_len)? {
            Ok(input) => input,
            Err(errno) => return Ok(errno),
        };
        let result = with_wasi_host_u32(&caller, |host| host.path_create_directory(dirfd, &path))?;
        return Ok(live_unit_result(result));
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::host::fs) fn path_filestat_set_times(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    flags: i32,
    path_ptr: i32,
    path_len: i32,
    atim: i64,
    mtim: i64,
    fstflags: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        if unsupported_lookupflags(flags) {
            return Ok(ERRNO_NOTCAPABLE);
        }
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let fstflags = match preview1_u16_filestat_flags(fstflags) {
            Ok(flags) => flags,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let path = read_checked_live_path(&caller, path_ptr, path_len)?;
        let result = with_wasi_host_u32(&caller, |host| {
            host.path_filestat_set_times(
                dirfd,
                flags.cast_unsigned(),
                &path,
                atim.cast_unsigned(),
                mtim.cast_unsigned(),
                fstflags,
            )
        })?;
        return Ok(live_unit_result(result));
    }
    if unsupported_lookupflags(flags) {
        return Ok(ERRNO_NOTCAPABLE);
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

pub(in crate::host::fs) fn path_remove_directory(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let (dirfd, path) = match live_path_input(&caller, dirfd, path_ptr, path_len)? {
            Ok(input) => input,
            Err(errno) => return Ok(errno),
        };
        let result = with_wasi_host_u32(&caller, |host| host.path_remove_directory(dirfd, &path))?;
        return Ok(live_unit_result(result));
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

pub(in crate::host::fs) fn path_unlink_file(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let (dirfd, path) = match live_path_input(&caller, dirfd, path_ptr, path_len)? {
            Ok(input) => input,
            Err(errno) => return Ok(errno),
        };
        let result = with_wasi_host_u32(&caller, |host| host.path_unlink_file(dirfd, &path))?;
        return Ok(live_unit_result(result));
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}
