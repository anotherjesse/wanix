use super::super::{
    ERRNO_BADF, ERRNO_INVAL, ERRNO_NOSYS, ERRNO_NOTCAPABLE, ERRNO_SUCCESS, HostState,
    QuickJsWasiWhence, WHENCE_CUR, WHENCE_END, WHENCE_SET, caller_memory, with_wasi_host,
};
use crate::guest::guest_offset;
use wasmtime::Caller;

pub(in crate::host::fs) fn fd_seek(
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
        return write_live_seek_result(&mut caller, result_ptr, result);
    }
    virtual_fd_seek(caller, fd, offset, whence, result_ptr)
}

fn write_live_seek_result(
    caller: &mut Caller<'_, HostState>,
    result_ptr: i32,
    result: Result<u64, super::super::QuickJsWasiErrno>,
) -> wasmtime::Result<i32> {
    let next = match result {
        Ok(next) => next,
        Err(errno) => return Ok(errno.preview1_result()),
    };
    write_position_result(caller, result_ptr, next)?;
    Ok(ERRNO_SUCCESS)
}

fn virtual_fd_seek(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    offset: i64,
    whence: i32,
    result_ptr: i32,
) -> wasmtime::Result<i32> {
    let input = match virtual_seek_input(&caller, fd)? {
        Ok(input) => input,
        Err(errno) => return Ok(errno),
    };
    let next = match virtual_seek_offset(input.current, input.len, offset, whence)? {
        Ok(next) => next,
        Err(errno) => return Ok(errno),
    };
    write_position_result(&mut caller, result_ptr, next)?;
    if let Some(file) = caller.data_mut().virtual_file_mut(fd) {
        file.offset = next;
    }
    Ok(ERRNO_SUCCESS)
}

struct VirtualSeekInput {
    current: u64,
    len: usize,
}

fn virtual_seek_input(
    caller: &Caller<'_, HostState>,
    fd: i32,
) -> wasmtime::Result<Result<VirtualSeekInput, i32>> {
    let Some(file) = caller.data().virtual_file(fd) else {
        return Ok(Err(ERRNO_BADF));
    };
    if file.rights_base & super::super::RIGHT_FD_SEEK == 0 {
        return Ok(Err(ERRNO_NOTCAPABLE));
    }
    Ok(Ok(VirtualSeekInput {
        current: file.offset,
        len: file.bytes.len(),
    }))
}

fn virtual_seek_offset(
    current: u64,
    len: usize,
    offset: i64,
    whence: i32,
) -> wasmtime::Result<Result<u64, i32>> {
    let base = match whence {
        WHENCE_SET => 0_i128,
        WHENCE_CUR => i128::from(current),
        WHENCE_END => i128::try_from(len)
            .map_err(|_| wasmtime::Error::msg("virtual file length exceeds i128"))?,
        _ => return Ok(Err(ERRNO_INVAL)),
    };
    let next = base + i128::from(offset);
    if !(0..=i128::from(u64::MAX)).contains(&next) {
        return Ok(Err(ERRNO_INVAL));
    }
    let next = u64::try_from(next)
        .map_err(|_| wasmtime::Error::msg("validated seek offset did not fit u64"))?;
    Ok(Ok(next))
}

pub(in crate::host::fs) fn fd_tell(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    result_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_tell(fd))? {
        return write_live_tell_result(&mut caller, result_ptr, result);
    }

    let (current, rights_base) = match caller.data().virtual_file(fd) {
        Some(file) => (file.offset, file.rights_base),
        None => return Ok(ERRNO_BADF),
    };
    if rights_base & super::super::RIGHT_FD_TELL == 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }

    write_position_result(&mut caller, result_ptr, current)?;
    Ok(ERRNO_SUCCESS)
}

fn write_live_tell_result(
    caller: &mut Caller<'_, HostState>,
    result_ptr: i32,
    result: Result<u64, super::super::QuickJsWasiErrno>,
) -> wasmtime::Result<i32> {
    let current = match result {
        Ok(current) => current,
        Err(errno) => return Ok(errno.preview1_result()),
    };
    write_position_result(caller, result_ptr, current)?;
    Ok(ERRNO_SUCCESS)
}

fn write_position_result(
    caller: &mut Caller<'_, HostState>,
    result_ptr: i32,
    position: u64,
) -> wasmtime::Result<()> {
    let memory = caller_memory(caller)?;
    memory.write(caller, guest_offset(result_ptr), &position.to_le_bytes())?;
    Ok(())
}

pub(in crate::host::fs) fn fd_close(
    mut caller: Caller<'_, HostState>,
    fd: i32,
) -> wasmtime::Result<i32> {
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
