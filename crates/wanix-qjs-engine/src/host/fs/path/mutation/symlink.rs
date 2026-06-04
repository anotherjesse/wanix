use super::super::super::{
    ERRNO_BADF, ERRNO_SUCCESS, HostState, caller_memory, checked_wasi_path_len, preview1_fd,
    read_guest_path, with_wasi_host_u32,
};
use super::super::unsupported_path_mutation;
use wasmtime::Caller;

pub(in crate::host::fs) fn path_symlink(
    caller: Caller<'_, HostState>,
    old_path_ptr: i32,
    old_path_len: i32,
    dirfd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        live_path_symlink(
            caller,
            old_path_ptr,
            old_path_len,
            dirfd,
            new_path_ptr,
            new_path_len,
        )
    } else {
        virtual_path_symlink(
            caller,
            old_path_ptr,
            old_path_len,
            dirfd,
            new_path_ptr,
            new_path_len,
        )
    }
}

fn live_path_symlink(
    caller: Caller<'_, HostState>,
    old_path_ptr: i32,
    old_path_len: i32,
    dirfd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
) -> wasmtime::Result<i32> {
    let dirfd = match preview1_fd(dirfd) {
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
        host.path_symlink(&old_path, dirfd, &new_path)
    })?
    else {
        return Ok(ERRNO_BADF);
    };
    match result {
        Ok(()) => Ok(ERRNO_SUCCESS),
        Err(errno) => Ok(errno.preview1_result()),
    }
}

fn virtual_path_symlink(
    caller: Caller<'_, HostState>,
    old_path_ptr: i32,
    old_path_len: i32,
    dirfd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
) -> wasmtime::Result<i32> {
    let old_path_len = match checked_wasi_path_len(old_path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(&caller)?;
    read_guest_path(&memory, &caller, old_path_ptr, old_path_len)?;
    unsupported_path_mutation(&caller, dirfd, new_path_ptr, new_path_len)
}
