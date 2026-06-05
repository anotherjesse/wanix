use super::{
    ERRNO_BADF, ERRNO_NOSYS, ERRNO_SUCCESS, HostState, LOOKUPFLAGS_SYMLINK_FOLLOW, WASI_U32_SIZE,
    caller_memory, checked_wasi_path_len, guest_len, guest_range, preview1_fd, read_guest_path,
    with_wasi_host, with_wasi_host_u32,
};
use crate::guest::guest_offset;
use wasmtime::{Caller, Memory};

mod mutation;
mod stat;
pub(super) use mutation::{
    path_create_directory, path_filestat_set_times, path_remove_directory, path_rename,
    path_symlink, path_unlink_file,
};
pub(super) use stat::path_filestat_get;

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
        let guest = ReadlinkGuestInput {
            dirfd,
            path_ptr,
            path_len,
            buf_ptr,
            buf_len,
            bufused_ptr,
        };
        let input = match live_readlink_input(&caller, guest)? {
            Ok(input) => input,
            Err(errno) => return Ok(errno),
        };
        let Some(result) =
            with_wasi_host_u32(&caller, |host| host.path_readlink(input.dirfd, &input.path))?
        else {
            return Ok(ERRNO_BADF);
        };
        let target = match result {
            Ok(target) => target,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        write_readlink_result(&input.memory, &mut caller, &input.output, &target)?;
        return Ok(ERRNO_SUCCESS);
    }

    let errno = unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)?;
    if errno != ERRNO_NOSYS {
        return Ok(errno);
    }
    Ok(ERRNO_NOSYS)
}

struct ReadlinkGuestInput {
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
    buf_ptr: i32,
    buf_len: i32,
    bufused_ptr: i32,
}

struct ReadlinkOutput {
    buf_ptr: i32,
    buf_len: usize,
    bufused_ptr: i32,
}

struct LiveReadlinkInput {
    memory: Memory,
    dirfd: u32,
    path: Vec<u8>,
    output: ReadlinkOutput,
}

fn live_readlink_input(
    caller: &Caller<'_, HostState>,
    guest: ReadlinkGuestInput,
) -> wasmtime::Result<Result<LiveReadlinkInput, i32>> {
    let dirfd = match preview1_fd(guest.dirfd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(Err(errno)),
    };
    let path_len = match checked_wasi_path_len(guest.path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(Err(errno)),
    };
    let buf_len = guest_len(guest.buf_len)?;
    let memory = caller_memory(caller)?;
    guest_range(&memory, caller, guest_offset(guest.buf_ptr), buf_len)?;
    guest_range(
        &memory,
        caller,
        guest_offset(guest.bufused_ptr),
        WASI_U32_SIZE,
    )?;
    let path = read_guest_path(&memory, caller, guest.path_ptr, path_len)?;
    Ok(Ok(LiveReadlinkInput {
        memory,
        dirfd,
        path,
        output: ReadlinkOutput {
            buf_ptr: guest.buf_ptr,
            buf_len,
            bufused_ptr: guest.bufused_ptr,
        },
    }))
}

fn write_readlink_result(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    output: &ReadlinkOutput,
    target: &[u8],
) -> wasmtime::Result<()> {
    let count = target.len().min(output.buf_len);
    memory.write(&mut *caller, guest_offset(output.buf_ptr), &target[..count])?;
    let count = u32::try_from(count)
        .map_err(|_| wasmtime::Error::msg("path_readlink byte count exceeds u32"))?;
    memory.write(
        &mut *caller,
        guest_offset(output.bufused_ptr),
        &count.to_le_bytes(),
    )?;
    Ok(())
}
