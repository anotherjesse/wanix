use super::super::super::{
    ERRNO_NOSYS, HostState, caller_memory, checked_wasi_path_len, preview1_fd, read_guest_path,
    with_wasi_host_u32,
};
use super::{live_unit_result, unsupported_path_mutation};
use wasmtime::Caller;

pub(in crate::host::fs) fn path_rename(
    caller: Caller<'_, HostState>,
    old_fd: i32,
    old_path_ptr: i32,
    old_path_len: i32,
    new_fd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
) -> wasmtime::Result<i32> {
    let input = PathRenameInput {
        old_fd,
        old_path_ptr,
        old_path_len,
        new_fd,
        new_path_ptr,
        new_path_len,
    };
    if caller.data().wasi_host().is_some() {
        return live_path_rename(&caller, input);
    }
    virtual_path_rename(&caller, input)
}

#[derive(Clone, Copy)]
struct PathRenameInput {
    old_fd: i32,
    old_path_ptr: i32,
    old_path_len: i32,
    new_fd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
}

fn live_path_rename(
    caller: &Caller<'_, HostState>,
    input: PathRenameInput,
) -> wasmtime::Result<i32> {
    let old_fd = match preview1_fd(input.old_fd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(errno),
    };
    let new_fd = match preview1_fd(input.new_fd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(errno),
    };
    let old_path_len = match checked_wasi_path_len(input.old_path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(errno),
    };
    let new_path_len = match checked_wasi_path_len(input.new_path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(caller)?;
    let old_path = read_guest_path(&memory, caller, input.old_path_ptr, old_path_len)?;
    let new_path = read_guest_path(&memory, caller, input.new_path_ptr, new_path_len)?;
    let result = with_wasi_host_u32(caller, |host| {
        host.path_rename(old_fd, &old_path, new_fd, &new_path)
    })?;
    Ok(live_unit_result(result))
}

fn virtual_path_rename(
    caller: &Caller<'_, HostState>,
    input: PathRenameInput,
) -> wasmtime::Result<i32> {
    let old_errno =
        unsupported_path_mutation(caller, input.old_fd, input.old_path_ptr, input.old_path_len)?;
    if old_errno != ERRNO_NOSYS {
        return Ok(old_errno);
    }
    let new_errno =
        unsupported_path_mutation(caller, input.new_fd, input.new_path_ptr, input.new_path_len)?;
    if new_errno != ERRNO_NOSYS {
        return Ok(new_errno);
    }
    Ok(ERRNO_NOSYS)
}
