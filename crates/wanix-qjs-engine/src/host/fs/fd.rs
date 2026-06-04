use super::{
    ERRNO_BADF, ERRNO_INVAL, ERRNO_NOSYS, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, HostState,
    PREOPEN_ROOT_PATH, QuickJsWasiWhence, WASI_U32_SIZE, WHENCE_CUR, WHENCE_END, WHENCE_SET,
    caller_memory, guest_len, guest_range, preview1_fd, with_wasi_host, with_wasi_host_u32,
    write_prestat, write_wasi_direntries,
};
use crate::guest::guest_offset;
use wasmtime::Caller;

mod stat;
pub(super) use stat::{
    fd_fdstat_get, fd_fdstat_set_flags, fd_filestat_get, fd_filestat_set_size,
    fd_filestat_set_times,
};

pub(super) fn fd_prestat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    prestat_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_prestat_get(fd))? {
        let prestat = match result {
            Ok(prestat) => prestat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        write_prestat(&memory, &mut caller, prestat_ptr, &prestat)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(fd) {
        return Ok(ERRNO_BADF);
    }
    let memory = caller_memory(&caller)?;
    let mut prestat = [0u8; super::PRESTAT_SIZE];
    prestat[4..8].copy_from_slice(&1_u32.to_le_bytes());
    memory.write(&mut caller, guest_offset(prestat_ptr), &prestat)?;
    Ok(ERRNO_SUCCESS)
}

pub(super) fn fd_prestat_dir_name(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_prestat_get(fd))? {
        let prestat = match result {
            Ok(prestat) => prestat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let path_len = guest_len(path_len)?;
        let path = prestat.dir_name().as_bytes();
        if path_len < path.len() {
            return Ok(ERRNO_INVAL);
        }
        let memory = caller_memory(&caller)?;
        memory.write(&mut caller, guest_offset(path_ptr), path)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(fd) {
        return Ok(ERRNO_BADF);
    }
    let path_len = guest_len(path_len)?;
    if path_len < PREOPEN_ROOT_PATH.len() {
        return Ok(ERRNO_INVAL);
    }
    let memory = caller_memory(&caller)?;
    memory.write(&mut caller, guest_offset(path_ptr), PREOPEN_ROOT_PATH)?;
    Ok(ERRNO_SUCCESS)
}

pub(super) fn fd_readdir(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    buf_ptr: i32,
    buf_len: i32,
    cookie: i64,
    bufused_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let fd = match preview1_fd(fd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let buf_len = guest_len(buf_len)?;
        let memory = caller_memory(&caller)?;
        let buf_range = guest_range(&memory, &caller, guest_offset(buf_ptr), buf_len)?;
        guest_range(&memory, &caller, guest_offset(bufused_ptr), WASI_U32_SIZE)?;

        let Some(result) = with_wasi_host_u32(&caller, |host| host.fd_readdir(fd))? else {
            return Ok(ERRNO_BADF);
        };
        let entries = match result {
            Ok(entries) => entries,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let used = write_wasi_direntries(
            &memory,
            &mut caller,
            buf_range.start,
            buf_len,
            cookie.cast_unsigned(),
            &entries,
        )?;
        let used = u32::try_from(used)
            .map_err(|_| wasmtime::Error::msg("fd_readdir byte count exceeds u32"))?;
        memory.write(&mut caller, guest_offset(bufused_ptr), &used.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    if caller.data().is_virtual_preopen_fd(fd) {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

pub(super) fn fd_seek(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    offset: i64,
    whence: i32,
    result_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| {
        let whence = QuickJsWasiWhence::from_preview1(whence)?;
        host.fd_seek(fd, offset, whence)
    })? {
        let next = match result {
            Ok(next) => next,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        memory.write(&mut caller, guest_offset(result_ptr), &next.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    let (current, len, rights_base) = match caller.data().virtual_file(fd) {
        Some(file) => (file.offset, file.bytes.len(), file.rights_base),
        None => return Ok(ERRNO_BADF),
    };
    if rights_base & super::RIGHT_FD_SEEK == 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let base = match whence {
        WHENCE_SET => 0_i128,
        WHENCE_CUR => i128::from(current),
        WHENCE_END => i128::try_from(len)
            .map_err(|_| wasmtime::Error::msg("virtual file length exceeds i128"))?,
        _ => return Ok(ERRNO_INVAL),
    };
    let next = base + i128::from(offset);
    if !(0..=i128::from(u64::MAX)).contains(&next) {
        return Ok(ERRNO_INVAL);
    }
    let next = u64::try_from(next)
        .map_err(|_| wasmtime::Error::msg("validated seek offset did not fit u64"))?;

    let memory = caller_memory(&caller)?;
    memory.write(&mut caller, guest_offset(result_ptr), &next.to_le_bytes())?;
    if let Some(file) = caller.data_mut().virtual_file_mut(fd) {
        file.offset = next;
    }
    Ok(ERRNO_SUCCESS)
}

pub(super) fn fd_tell(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    result_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_tell(fd))? {
        let current = match result {
            Ok(current) => current,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        memory.write(
            &mut caller,
            guest_offset(result_ptr),
            &current.to_le_bytes(),
        )?;
        return Ok(ERRNO_SUCCESS);
    }

    let (current, rights_base) = match caller.data().virtual_file(fd) {
        Some(file) => (file.offset, file.rights_base),
        None => return Ok(ERRNO_BADF),
    };
    if rights_base & super::RIGHT_FD_TELL == 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let memory = caller_memory(&caller)?;
    memory.write(
        &mut caller,
        guest_offset(result_ptr),
        &current.to_le_bytes(),
    )?;
    Ok(ERRNO_SUCCESS)
}

pub(super) fn fd_close(mut caller: Caller<'_, HostState>, fd: i32) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_close(fd))? {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }

    if caller.data_mut().close_virtual_file(fd) {
        Ok(ERRNO_SUCCESS)
    } else if fd == 1 || fd == 2 || caller.data().is_virtual_preopen_fd(fd) {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}
