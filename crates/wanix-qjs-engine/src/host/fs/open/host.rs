use super::{OpenResult, PathOpenArgs, preview1, require_opened_fd_range, write_opened_u32_fd};
use crate::host::fs::path::unsupported_lookupflags;
use crate::host::fs::{
    ERRNO_BADF, ERRNO_NOTCAPABLE, HostState, caller_memory, checked_wasi_path_len, preview1_fd,
    preview1_u16_flags, read_guest_path, with_wasi_host_u32,
};
use wasmtime::Caller;

pub(super) fn open_wasi_host_path(
    mut caller: Caller<'_, HostState>,
    args: PathOpenArgs,
) -> wasmtime::Result<i32> {
    super::preview1_open_result(open_wasi_host_path_result(&mut caller, args))
}

fn open_wasi_host_path_result(
    caller: &mut Caller<'_, HostState>,
    args: PathOpenArgs,
) -> OpenResult<()> {
    let request = preview1(HostPathOpenRequest::from_preview1(args))?;
    let (memory, path) = read_host_open_path(caller, args)?;
    write_host_opened_fd(&memory, caller, args.opened_fd_ptr, request, &path)?;
    Ok(())
}

fn read_host_open_path(
    caller: &mut Caller<'_, HostState>,
    args: PathOpenArgs,
) -> OpenResult<(wasmtime::Memory, Vec<u8>)> {
    let memory = caller_memory(caller)?;
    require_opened_fd_range(&memory, caller, args.opened_fd_ptr)?;
    let path_len = checked_open_path_len(args.path_len)?;
    let path = read_guest_path(&memory, caller, args.path_ptr, path_len)?;
    Ok((memory, path))
}

struct HostPathOpenRequest {
    dirfd: u32,
    dirflags: u32,
    oflags: u16,
    fs_rights_base: u64,
    fs_rights_inheriting: u64,
    fdflags: u16,
}

impl HostPathOpenRequest {
    fn from_preview1(args: PathOpenArgs) -> Result<Self, i32> {
        if unsupported_lookupflags(args.dirflags) {
            return Err(ERRNO_NOTCAPABLE);
        }
        Ok(Self {
            dirfd: preview1_fd(args.dirfd)?,
            dirflags: args.dirflags.cast_unsigned(),
            oflags: preview1_u16_flags(args.oflags)?,
            fs_rights_base: args.fs_rights_base.cast_unsigned(),
            fs_rights_inheriting: args.fs_rights_inheriting.cast_unsigned(),
            fdflags: preview1_u16_flags(args.fdflags)?,
        })
    }
}

fn call_wasi_host_path_open(
    caller: &Caller<'_, HostState>,
    request: HostPathOpenRequest,
    path: &[u8],
) -> wasmtime::Result<Result<u32, i32>> {
    let Some(result) = with_wasi_host_u32(caller, |host| {
        host.path_open(
            request.dirfd,
            request.dirflags,
            path,
            request.oflags,
            request.fs_rights_base,
            request.fs_rights_inheriting,
            request.fdflags,
        )
    })?
    else {
        return Ok(Err(ERRNO_BADF));
    };
    Ok(result.map_err(|errno| errno.preview1_result()))
}

fn write_host_opened_fd(
    memory: &wasmtime::Memory,
    caller: &mut Caller<'_, HostState>,
    opened_fd_ptr: i32,
    request: HostPathOpenRequest,
    path: &[u8],
) -> OpenResult<()> {
    let fd = preview1(call_wasi_host_path_open(caller, request, path)?)?;
    write_opened_u32_fd(memory, caller, opened_fd_ptr, fd)?;
    Ok(())
}

fn checked_open_path_len(path_len: i32) -> OpenResult<usize> {
    preview1(checked_wasi_path_len(path_len)?)
}
