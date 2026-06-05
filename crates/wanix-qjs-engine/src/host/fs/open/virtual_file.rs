use super::{OpenResult, PathOpenArgs, preview1, require_opened_fd_range, write_opened_i32_fd};
use crate::host::fs::path::unsupported_lookupflags;
use crate::host::fs::{
    ALLOWED_FILE_RIGHTS, ERRNO_BADF, ERRNO_INVAL, ERRNO_NOENT, ERRNO_NOTCAPABLE, HostState,
    caller_memory, read_absolute_virtual_path,
};
use wasmtime::Caller;

pub(super) fn open_virtual_file(
    mut caller: Caller<'_, HostState>,
    args: PathOpenArgs,
) -> wasmtime::Result<i32> {
    super::preview1_open_result(open_virtual_file_result(&mut caller, args))
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
