use super::{
    ERRNO_BADF, ERRNO_NOENT, ERRNO_NOSYS, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, FILESTAT_SIZE,
    FILETYPE_DIRECTORY, FILETYPE_REGULAR_FILE, FilestatFields, HostState,
    LOOKUPFLAGS_SYMLINK_FOLLOW, WASI_U32_SIZE, caller_memory, checked_wasi_path_len, guest_len,
    guest_range, preview1_fd, read_absolute_virtual_path, read_guest_path, with_wasi_host,
    with_wasi_host_u32, write_filestat, write_wasi_filestat,
};
use crate::guest::guest_offset;
use wasmtime::Caller;

mod mutation;
pub(super) use mutation::{
    path_create_directory, path_filestat_set_times, path_remove_directory, path_rename,
    path_symlink, path_unlink_file,
};

pub(in crate::host::fs) fn unsupported_lookupflags(flags: i32) -> bool {
    flags.cast_unsigned() & !LOOKUPFLAGS_SYMLINK_FOLLOW != 0
}

fn unsupported_path_mutation(
    caller: &Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if let Err(errno) = preview1_fd(dirfd) {
        return Ok(errno);
    }
    if let Some(result) = with_wasi_host(caller, dirfd, |host, fd| host.fd_fdstat_get(fd))? {
        if let Err(errno) = result {
            return Ok(errno.preview1_result());
        }
    } else if !caller.data().is_virtual_preopen_fd(dirfd) {
        return Ok(ERRNO_BADF);
    }
    let path_len = match checked_wasi_path_len(path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(caller)?;
    let _path = read_guest_path(&memory, caller, path_ptr, path_len)?;
    Ok(ERRNO_NOSYS)
}

pub(super) fn path_filestat_get(
    mut caller: Caller<'_, HostState>,
    dirfd: i32,
    flags: i32,
    path_ptr: i32,
    path_len: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        if unsupported_lookupflags(flags) {
            return Ok(ERRNO_NOTCAPABLE);
        }
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        guest_range(&memory, &caller, guest_offset(stat_ptr), FILESTAT_SIZE)?;
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_filestat_get(dirfd, flags.cast_unsigned(), &path)
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        let stat = match result {
            Ok(stat) => stat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        write_wasi_filestat(&memory, &mut caller, stat_ptr, stat)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(dirfd) {
        return Ok(ERRNO_BADF);
    }
    if unsupported_lookupflags(flags) {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let memory = caller_memory(&caller)?;
    guest_range(&memory, &caller, guest_offset(stat_ptr), FILESTAT_SIZE)?;
    let path = match read_absolute_virtual_path(&memory, &caller, path_ptr, path_len)? {
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
        &mut caller,
        stat_ptr,
        FilestatFields::new(filetype, size),
    )?;
    Ok(ERRNO_SUCCESS)
}

pub(super) fn path_readlink(
    mut caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
    buf_ptr: i32,
    buf_len: i32,
    bufused_ptr: i32,
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
        let buf_len = guest_len(buf_len)?;
        let memory = caller_memory(&caller)?;
        guest_range(&memory, &caller, guest_offset(buf_ptr), buf_len)?;
        guest_range(&memory, &caller, guest_offset(bufused_ptr), WASI_U32_SIZE)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| host.path_readlink(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        let target = match result {
            Ok(target) => target,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let count = target.len().min(buf_len);
        memory.write(&mut caller, guest_offset(buf_ptr), &target[..count])?;
        let count = u32::try_from(count)
            .map_err(|_| wasmtime::Error::msg("path_readlink byte count exceeds u32"))?;
        memory.write(&mut caller, guest_offset(bufused_ptr), &count.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    let errno = unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)?;
    if errno != ERRNO_NOSYS {
        return Ok(errno);
    }
    Ok(ERRNO_NOSYS)
}
