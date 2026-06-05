use super::super::super::{
    HostState, caller_memory, checked_wasi_path_len, preview1_fd, read_guest_path,
    with_wasi_host_u32,
};
use super::super::unsupported_path_mutation;
use super::live_unit_result;
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
    let input = match read_live_symlink_input(
        &caller,
        LiveSymlinkGuestInput {
            target_ptr: old_path_ptr,
            target_len: old_path_len,
            dirfd,
            path_ptr: new_path_ptr,
            path_len: new_path_len,
        },
    )? {
        Ok(input) => input,
        Err(errno) => return Ok(errno),
    };

    let result = with_wasi_host_u32(&caller, |host| {
        host.path_symlink(&input.target, input.dirfd, &input.path)
    })?;
    Ok(live_unit_result(result))
}

struct LiveSymlinkGuestInput {
    target_ptr: i32,
    target_len: i32,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
}

struct LiveSymlinkInput {
    target: Vec<u8>,
    dirfd: u32,
    path: Vec<u8>,
}

fn read_live_symlink_input(
    caller: &Caller<'_, HostState>,
    guest: LiveSymlinkGuestInput,
) -> wasmtime::Result<Result<LiveSymlinkInput, i32>> {
    let dirfd = match preview1_fd(guest.dirfd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(Err(errno)),
    };
    let target_len = match checked_wasi_path_len(guest.target_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(Err(errno)),
    };
    let path_len = match checked_wasi_path_len(guest.path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(Err(errno)),
    };
    let memory = caller_memory(caller)?;
    let target = read_guest_path(&memory, caller, guest.target_ptr, target_len)?;
    let path = read_guest_path(&memory, caller, guest.path_ptr, path_len)?;
    Ok(Ok(LiveSymlinkInput {
        target,
        dirfd,
        path,
    }))
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
