use super::{ERRNO_SUCCESS, HostState, WASI_U32_SIZE};
use crate::guest::guest_offset;
use wasmtime::Caller;

mod host;
mod virtual_file;

use host::open_wasi_host_path;
use virtual_file::open_virtual_file;

#[derive(Clone, Copy)]
struct PathOpenArgs {
    dirfd: i32,
    dirflags: i32,
    path_ptr: i32,
    path_len: i32,
    oflags: i32,
    fs_rights_base: i64,
    fs_rights_inheriting: i64,
    fdflags: i32,
    opened_fd_ptr: i32,
}

enum OpenError {
    Errno(i32),
    Trap(wasmtime::Error),
}

type OpenResult<T> = Result<T, OpenError>;

impl From<wasmtime::Error> for OpenError {
    fn from(error: wasmtime::Error) -> Self {
        Self::Trap(error)
    }
}

impl From<wasmtime::MemoryAccessError> for OpenError {
    fn from(error: wasmtime::MemoryAccessError) -> Self {
        Self::Trap(error.into())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn path_open(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    dirflags: i32,
    path_ptr: i32,
    path_len: i32,
    oflags: i32,
    fs_rights_base: i64,
    fs_rights_inheriting: i64,
    fdflags: i32,
    opened_fd_ptr: i32,
) -> wasmtime::Result<i32> {
    let args = PathOpenArgs {
        dirfd,
        dirflags,
        path_ptr,
        path_len,
        oflags,
        fs_rights_base,
        fs_rights_inheriting,
        fdflags,
        opened_fd_ptr,
    };
    if caller.data().wasi_host().is_some() {
        open_wasi_host_path(caller, args)
    } else {
        open_virtual_file(caller, args)
    }
}

fn write_opened_u32_fd(
    memory: &wasmtime::Memory,
    caller: &mut Caller<'_, HostState>,
    opened_fd_ptr: i32,
    fd: u32,
) -> wasmtime::Result<()> {
    memory
        .write(caller, guest_offset(opened_fd_ptr), &fd.to_le_bytes())
        .map_err(Into::into)
}

fn write_opened_i32_fd(
    memory: &wasmtime::Memory,
    caller: &mut Caller<'_, HostState>,
    opened_fd_ptr: i32,
    fd: i32,
) -> wasmtime::Result<()> {
    memory
        .write(caller, guest_offset(opened_fd_ptr), &fd.to_le_bytes())
        .map_err(Into::into)
}

fn preview1<T>(result: Result<T, i32>) -> OpenResult<T> {
    result.map_err(OpenError::Errno)
}

fn preview1_open_result(result: OpenResult<()>) -> wasmtime::Result<i32> {
    match result {
        Ok(()) => Ok(ERRNO_SUCCESS),
        Err(OpenError::Errno(errno)) => Ok(errno),
        Err(OpenError::Trap(error)) => Err(error),
    }
}

fn require_opened_fd_range(
    memory: &wasmtime::Memory,
    caller: &Caller<'_, HostState>,
    opened_fd_ptr: i32,
) -> OpenResult<()> {
    super::guest_range(memory, caller, guest_offset(opened_fd_ptr), WASI_U32_SIZE)?;
    Ok(())
}
