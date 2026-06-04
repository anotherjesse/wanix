use super::super::{
    ERRNO_BADF, ERRNO_NOSYS, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, HostState, caller_memory,
    checked_wasi_path_len, preview1_fd, preview1_u16_filestat_flags, read_guest_path,
    with_wasi_host_u32,
};
use super::{unsupported_lookupflags, unsupported_path_mutation};
use wasmtime::Caller;

mod symlink;
pub(in crate::host::fs) use symlink::path_symlink;

pub(in crate::host::fs) fn path_create_directory(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) =
            with_wasi_host_u32(&caller, |host| host.path_create_directory(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
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
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_filestat_set_times(
                dirfd,
                flags.cast_unsigned(),
                &path,
                atim.cast_unsigned(),
                mtim.cast_unsigned(),
                fstflags,
            )
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
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
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) =
            with_wasi_host_u32(&caller, |host| host.path_remove_directory(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

pub(in crate::host::fs) fn path_rename(
    caller: Caller<'_, HostState>,
    old_fd: i32,
    old_path_ptr: i32,
    old_path_len: i32,
    new_fd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let old_fd = match preview1_fd(old_fd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let new_fd = match preview1_fd(new_fd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let old_path_len = match checked_wasi_path_len(old_path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let new_path_len = match checked_wasi_path_len(new_path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let old_path = read_guest_path(&memory, &caller, old_path_ptr, old_path_len)?;
        let new_path = read_guest_path(&memory, &caller, new_path_ptr, new_path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_rename(old_fd, &old_path, new_fd, &new_path)
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }

    let old_errno = unsupported_path_mutation(&caller, old_fd, old_path_ptr, old_path_len)?;
    if old_errno != ERRNO_NOSYS {
        return Ok(old_errno);
    }
    let new_errno = unsupported_path_mutation(&caller, new_fd, new_path_ptr, new_path_len)?;
    if new_errno != ERRNO_NOSYS {
        return Ok(new_errno);
    }
    Ok(ERRNO_NOSYS)
}

pub(in crate::host::fs) fn path_unlink_file(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| host.path_unlink_file(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}
