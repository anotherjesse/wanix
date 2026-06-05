use super::super::{
    ERRNO_BADF, ERRNO_NOENT, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, FILESTAT_SIZE, FILETYPE_DIRECTORY,
    FILETYPE_REGULAR_FILE, FilestatFields, HostState, caller_memory, checked_wasi_path_len,
    guest_range, preview1_fd, read_absolute_virtual_path, read_guest_path, with_wasi_host_u32,
    write_filestat, write_wasi_filestat,
};
use super::unsupported_lookupflags;
use crate::guest::guest_offset;
use wasmtime::Caller;

pub(in crate::host::fs) fn path_filestat_get(
    mut caller: Caller<'_, HostState>,
    dirfd: i32,
    flags: i32,
    path_ptr: i32,
    path_len: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
    let input = PathFilestatGetInput {
        dirfd,
        flags,
        path_ptr,
        path_len,
        stat_ptr,
    };
    if caller.data().wasi_host().is_some() {
        return live_path_filestat_get(&mut caller, input);
    }
    virtual_path_filestat_get(&mut caller, input)
}

#[derive(Clone, Copy)]
struct PathFilestatGetInput {
    dirfd: i32,
    flags: i32,
    path_ptr: i32,
    path_len: i32,
    stat_ptr: i32,
}

fn live_path_filestat_get(
    caller: &mut Caller<'_, HostState>,
    input: PathFilestatGetInput,
) -> wasmtime::Result<i32> {
    if unsupported_lookupflags(input.flags) {
        return Ok(ERRNO_NOTCAPABLE);
    }
    let dirfd = match preview1_fd(input.dirfd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(caller)?;
    guest_range(&memory, caller, guest_offset(input.stat_ptr), FILESTAT_SIZE)?;
    let path_len = match checked_wasi_path_len(input.path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(errno),
    };
    let path = read_guest_path(&memory, caller, input.path_ptr, path_len)?;
    let Some(result) = with_wasi_host_u32(caller, |host| {
        host.path_filestat_get(dirfd, input.flags.cast_unsigned(), &path)
    })?
    else {
        return Ok(ERRNO_BADF);
    };
    let stat = match result {
        Ok(stat) => stat,
        Err(errno) => return Ok(errno.preview1_result()),
    };
    write_wasi_filestat(&memory, caller, input.stat_ptr, stat)?;
    Ok(ERRNO_SUCCESS)
}

fn virtual_path_filestat_get(
    caller: &mut Caller<'_, HostState>,
    input: PathFilestatGetInput,
) -> wasmtime::Result<i32> {
    if !caller.data().is_virtual_preopen_fd(input.dirfd) {
        return Ok(ERRNO_BADF);
    }
    if unsupported_lookupflags(input.flags) {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let memory = caller_memory(caller)?;
    guest_range(&memory, caller, guest_offset(input.stat_ptr), FILESTAT_SIZE)?;
    let path = match read_absolute_virtual_path(&memory, caller, input.path_ptr, input.path_len)? {
        Ok(path) => path,
        Err(errno) => return Ok(errno),
    };
    let (filetype, size) = if let Some(bytes) = caller.data().config().read_only_virtual_file(&path)
    {
        let size = u64::try_from(bytes.len())
            .map_err(|_| wasmtime::Error::msg("virtual file length exceeds u64"))?;
        (FILETYPE_REGULAR_FILE, size)
    } else if caller
        .data()
        .config()
        .has_read_only_virtual_directory(&path)
    {
        (FILETYPE_DIRECTORY, 0)
    } else {
        return Ok(ERRNO_NOENT);
    };

    write_filestat(
        &memory,
        caller,
        input.stat_ptr,
        FilestatFields::new(filetype, size),
    )?;
    Ok(ERRNO_SUCCESS)
}
