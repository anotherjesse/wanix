use super::super::{
    ALLOWED_FILE_RIGHTS, ERRNO_BADF, ERRNO_NOSYS, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, FDSTAT_SIZE,
    FILETYPE_CHARACTER_DEVICE, FILETYPE_DIRECTORY, FILETYPE_REGULAR_FILE, FilestatFields,
    HostState, PREOPEN_ROOT_RIGHTS, RIGHT_FD_FILESTAT_GET, caller_memory,
    preview1_u16_filestat_flags, preview1_u16_flags, wasi_stdio_fd, with_wasi_host, write_filestat,
    write_wasi_fdstat, write_wasi_filestat,
};
use crate::guest::guest_offset;
use wasmtime::Caller;

pub(in crate::host::fs) fn fd_filestat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_filestat_get(fd))? {
        let stat = match result {
            Ok(stat) => stat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        write_wasi_filestat(&memory, &mut caller, stat_ptr, stat)?;
        return Ok(ERRNO_SUCCESS);
    }

    let (filetype, size) = if wasi_stdio_fd(fd).is_some() {
        (FILETYPE_CHARACTER_DEVICE, 0)
    } else if caller.data().is_virtual_preopen_fd(fd) {
        (FILETYPE_DIRECTORY, 0)
    } else if let Some(file) = caller.data().virtual_file(fd) {
        if file.rights_base & RIGHT_FD_FILESTAT_GET == 0 {
            return Ok(ERRNO_NOTCAPABLE);
        }
        let size = u64::try_from(file.bytes.len())
            .map_err(|_| wasmtime::Error::msg("virtual file length exceeds u64"))?;
        (FILETYPE_REGULAR_FILE, size)
    } else {
        return Ok(ERRNO_BADF);
    };

    let memory = caller_memory(&caller)?;
    write_filestat(
        &memory,
        &mut caller,
        stat_ptr,
        FilestatFields::new(filetype, size),
    )?;
    Ok(ERRNO_SUCCESS)
}

pub(in crate::host::fs) fn fd_filestat_set_times(
    caller: Caller<'_, HostState>,
    fd: i32,
    atim: i64,
    mtim: i64,
    fstflags: i32,
) -> wasmtime::Result<i32> {
    let fstflags = match preview1_u16_filestat_flags(fstflags) {
        Ok(flags) => flags,
        Err(errno) => return Ok(errno),
    };
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| {
        host.fd_filestat_set_times(fd, atim.cast_unsigned(), mtim.cast_unsigned(), fstflags)
    })? {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }
    if wasi_stdio_fd(fd).is_some()
        || caller.data().is_virtual_preopen_fd(fd)
        || caller.data().virtual_file(fd).is_some()
    {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

pub(in crate::host::fs) fn fd_filestat_set_size(
    caller: Caller<'_, HostState>,
    fd: i32,
    size: i64,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| {
        host.fd_filestat_set_size(fd, size.cast_unsigned())
    })? {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }
    if wasi_stdio_fd(fd).is_some()
        || caller.data().is_virtual_preopen_fd(fd)
        || caller.data().virtual_file(fd).is_some()
    {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

pub(in crate::host::fs) fn fd_fdstat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_fdstat_get(fd))? {
        let stat = match result {
            Ok(stat) => stat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        write_wasi_fdstat(&memory, &mut caller, stat_ptr, stat)?;
        return Ok(ERRNO_SUCCESS);
    }

    let (filetype, rights_base, rights_inheriting) = if wasi_stdio_fd(fd).is_some() {
        (FILETYPE_CHARACTER_DEVICE, 0, 0)
    } else if caller.data().is_virtual_preopen_fd(fd) {
        (FILETYPE_DIRECTORY, PREOPEN_ROOT_RIGHTS, ALLOWED_FILE_RIGHTS)
    } else if let Some(file) = caller.data().virtual_file(fd) {
        (FILETYPE_REGULAR_FILE, file.rights_base, 0)
    } else {
        return Ok(ERRNO_BADF);
    };

    let memory = caller_memory(&caller)?;
    let mut stat = [0u8; FDSTAT_SIZE];
    stat[0] = filetype;
    stat[8..16].copy_from_slice(&rights_base.to_le_bytes());
    stat[16..24].copy_from_slice(&rights_inheriting.to_le_bytes());
    memory.write(&mut caller, guest_offset(stat_ptr), &stat)?;
    Ok(ERRNO_SUCCESS)
}

pub(in crate::host::fs) fn fd_fdstat_set_flags(
    caller: Caller<'_, HostState>,
    fd: i32,
    flags: i32,
) -> wasmtime::Result<i32> {
    if let Err(errno) = super::super::preview1_fd(fd) {
        return Ok(errno);
    }
    let flags = match preview1_u16_flags(flags) {
        Ok(flags) => flags,
        Err(errno) => return Ok(errno),
    };
    if let Some(result) =
        with_wasi_host(&caller, fd, |host, fd| host.fd_fdstat_set_flags(fd, flags))?
    {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }
    if wasi_stdio_fd(fd).is_some()
        || caller.data().is_virtual_preopen_fd(fd)
        || caller.data().virtual_file(fd).is_some()
    {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}
