use super::{
    ALLOWED_FILE_RIGHTS, ERRNO_BADF, ERRNO_INVAL, ERRNO_NOENT, ERRNO_NOTCAPABLE, ERRNO_SUCCESS,
    HostState, WASI_U32_SIZE, caller_memory, checked_wasi_path_len, preview1_fd,
    preview1_u16_flags, read_absolute_virtual_path, read_guest_path, unsupported_lookupflags,
    with_wasi_host_u32,
};
use crate::guest::guest_offset;
use wasmtime::Caller;

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

fn open_wasi_host_path(
    mut caller: Caller<'_, HostState>,
    args: PathOpenArgs,
) -> wasmtime::Result<i32> {
    preview1_open_result(open_wasi_host_path_result(&mut caller, args))
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

fn open_virtual_file(
    mut caller: Caller<'_, HostState>,
    args: PathOpenArgs,
) -> wasmtime::Result<i32> {
    preview1_open_result(open_virtual_file_result(&mut caller, args))
}

fn open_virtual_file_result(
    caller: &mut Caller<'_, HostState>,
    args: PathOpenArgs,
) -> OpenResult<()> {
    let rights_base = preview1(validate_virtual_open(caller, args))?;
    let (memory, path) = read_virtual_open_path(caller, args)?;
    let fd = install_virtual_open(caller, path, rights_base)?;
    write_virtual_opened_fd(&memory, caller, args.opened_fd_ptr, fd)?;
    Ok(())
}

fn read_virtual_open_path(
    caller: &mut Caller<'_, HostState>,
    args: PathOpenArgs,
) -> OpenResult<(wasmtime::Memory, Vec<u8>)> {
    let memory = caller_memory(caller)?;
    require_opened_fd_range(&memory, caller, args.opened_fd_ptr)?;
    let path = preview1(read_absolute_virtual_path(
        &memory,
        caller,
        args.path_ptr,
        args.path_len,
    )?)?;
    Ok((memory, path))
}

fn install_virtual_open(
    caller: &mut Caller<'_, HostState>,
    path: Vec<u8>,
    rights_base: u64,
) -> OpenResult<i32> {
    preview1(ensure_virtual_file_exists(caller, &path))?;
    preview1(
        caller
            .data_mut()
            .open_virtual_file(path, rights_base)
            .ok_or(ERRNO_INVAL),
    )
}

fn validate_virtual_open(caller: &Caller<'_, HostState>, args: PathOpenArgs) -> Result<u64, i32> {
    if !caller.data().is_virtual_preopen_fd(args.dirfd) {
        return Err(ERRNO_BADF);
    }
    validate_virtual_open_flags(args)?;
    validate_virtual_rights(args)
}

fn validate_virtual_open_flags(args: PathOpenArgs) -> Result<(), i32> {
    if unsupported_lookupflags(args.dirflags) || args.oflags != 0 || args.fdflags != 0 {
        return Err(ERRNO_NOTCAPABLE);
    }
    Ok(())
}

fn validate_virtual_rights(args: PathOpenArgs) -> Result<u64, i32> {
    let rights_base = args.fs_rights_base.cast_unsigned();
    let rights_inheriting = args.fs_rights_inheriting.cast_unsigned();
    if rights_base & !ALLOWED_FILE_RIGHTS != 0 || rights_inheriting & !ALLOWED_FILE_RIGHTS != 0 {
        return Err(ERRNO_NOTCAPABLE);
    }
    Ok(rights_base)
}

fn ensure_virtual_file_exists(caller: &Caller<'_, HostState>, path: &[u8]) -> Result<(), i32> {
    caller
        .data()
        .config()
        .read_only_virtual_file(path)
        .map(|_| ())
        .ok_or(ERRNO_NOENT)
}

fn write_virtual_opened_fd(
    memory: &wasmtime::Memory,
    caller: &mut Caller<'_, HostState>,
    opened_fd_ptr: i32,
    fd: i32,
) -> wasmtime::Result<()> {
    let write = write_opened_i32_fd(memory, caller, opened_fd_ptr, fd);
    if write.is_err() {
        caller.data_mut().close_virtual_file(fd);
    }
    write
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

fn checked_open_path_len(path_len: i32) -> OpenResult<usize> {
    preview1(checked_wasi_path_len(path_len)?)
}

fn require_opened_fd_range(
    memory: &wasmtime::Memory,
    caller: &Caller<'_, HostState>,
    opened_fd_ptr: i32,
) -> OpenResult<()> {
    super::guest_range(memory, caller, guest_offset(opened_fd_ptr), WASI_U32_SIZE)?;
    Ok(())
}
