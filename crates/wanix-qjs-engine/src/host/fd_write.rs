use super::guest_memory::{
    capture_guest_buffer, guest_len, guest_offset_at, guest_range, write_guest_buffer,
};
use super::{ERRNO_BADF, ERRNO_SUCCESS, HostState, WasiStdioFd, caller_memory, wasi_stdio_fd};
use crate::guest::guest_offset;
use wasmtime::{Caller, Linker, Memory};

const WASI_U32_SIZE: usize = 4;
const WASI_IOV_SIZE: usize = 2 * WASI_U32_SIZE;
const WASI_IOV_LEN_OFFSET: usize = WASI_U32_SIZE;

pub(super) fn define_import(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, HostState>,
         fd: i32,
         iovs_ptr: i32,
         iovs_len: i32,
         nwritten_ptr: i32|
         -> wasmtime::Result<i32> {
            if caller.data().wasi_host().is_some() {
                return fd_write_with_wasi_host(caller, fd, iovs_ptr, iovs_len, nwritten_ptr);
            }

            let Some(fd) = wasi_stdio_fd(fd) else {
                return Ok(ERRNO_BADF);
            };

            let memory = caller_memory(&caller)?;
            let iovs_len = guest_len(iovs_len)?;
            guest_range(&memory, &caller, guest_offset(nwritten_ptr), WASI_U32_SIZE)?;
            let total_written = preflight_fd_write_iovs(&memory, &caller, iovs_ptr, iovs_len)?;
            reserve_fd_capture(&mut caller, fd, total_written)?;
            // The import is synchronous and does not reenter guest code, so the
            // second pass can reread descriptors without allocating a descriptor Vec.
            write_fd_iov_buffers(&mut caller, &memory, fd, iovs_ptr, iovs_len)?;

            memory.write(
                &mut caller,
                guest_offset(nwritten_ptr),
                &total_written.to_le_bytes(),
            )?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    Ok(())
}

fn fd_write_with_wasi_host(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    iovs_ptr: i32,
    iovs_len: i32,
    nwritten_ptr: i32,
) -> wasmtime::Result<i32> {
    let fd = match u32::try_from(fd) {
        Ok(fd) => fd,
        Err(_) => return Ok(ERRNO_BADF),
    };
    let memory = caller_memory(&caller)?;
    let iovs_len = guest_len(iovs_len)?;
    guest_range(&memory, &caller, guest_offset(nwritten_ptr), WASI_U32_SIZE)?;
    preflight_fd_write_iovs(&memory, &caller, iovs_ptr, iovs_len)?;

    let mut iovs = Vec::new();
    iovs.try_reserve_exact(iovs_len)
        .map_err(|_| wasmtime::Error::msg("fd_write iov allocation failed"))?;
    for index in 0..iovs_len {
        let iov = read_valid_fd_iov(&memory, &caller, iovs_ptr, index)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(iov.len)
            .map_err(|_| wasmtime::Error::msg("fd_write buffer allocation failed"))?;
        bytes.resize(iov.len, 0);
        memory.read(&caller, iov.ptr, &mut bytes)?;
        iovs.push(bytes);
    }

    let Some(host) = caller.data().wasi_host() else {
        return Ok(ERRNO_BADF);
    };
    let mut host = host
        .lock()
        .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
    let mut total_written = 0u32;
    for bytes in &iovs {
        let count = match host.fd_write(fd, bytes) {
            Ok(count) => count,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        if count > bytes.len() {
            return Err(wasmtime::Error::msg(
                "QuickJS WASI host returned oversized fd_write count",
            ));
        }
        total_written = checked_fd_write_total(
            total_written,
            u32::try_from(count)
                .map_err(|_| wasmtime::Error::msg("fd_write byte count exceeds u32"))?,
        )?;
        if count < bytes.len() {
            break;
        }
    }

    memory.write(
        &mut caller,
        guest_offset(nwritten_ptr),
        &total_written.to_le_bytes(),
    )?;
    Ok(ERRNO_SUCCESS)
}

#[derive(Debug)]
struct GuestIov {
    ptr: usize,
    len: usize,
    len_u32: u32,
}

fn preflight_fd_write_iovs(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    iovs_len: usize,
) -> wasmtime::Result<u32> {
    if iovs_len > 0 {
        guest_range(
            memory,
            caller,
            guest_offset_at(iovs_ptr, iovs_len - 1, WASI_IOV_SIZE)?,
            WASI_IOV_SIZE,
        )?;
    }

    let mut total_written = 0u32;
    for index in 0..iovs_len {
        let iov = read_valid_fd_iov(memory, caller, iovs_ptr, index)?;
        total_written = checked_fd_write_total(total_written, iov.len_u32)?;
    }
    Ok(total_written)
}

fn write_fd_iov_buffers(
    caller: &mut Caller<'_, HostState>,
    memory: &Memory,
    fd: WasiStdioFd,
    iovs_ptr: i32,
    iovs_len: usize,
) -> wasmtime::Result<()> {
    for index in 0..iovs_len {
        let iov = read_valid_fd_iov(memory, &*caller, iovs_ptr, index)?;
        write_fd_buffer(caller, memory, fd, iov.ptr, iov.len)?;
    }
    Ok(())
}

fn read_valid_fd_iov(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    index: usize,
) -> wasmtime::Result<GuestIov> {
    let iov_offset = guest_offset_at(iovs_ptr, index, WASI_IOV_SIZE)?;
    let mut iov = [0u8; WASI_IOV_SIZE];
    memory.read(caller, iov_offset, &mut iov)?;

    let ptr = read_iov_u32(&iov, 0);
    let len_u32 = read_iov_u32(&iov, WASI_IOV_LEN_OFFSET);
    let ptr = usize::try_from(ptr)
        .map_err(|_| wasmtime::Error::msg("guest iov pointer does not fit host usize"))?;
    let len = usize::try_from(len_u32)
        .map_err(|_| wasmtime::Error::msg("guest iov length does not fit host usize"))?;

    guest_range(memory, caller, ptr, len)?;
    Ok(GuestIov { ptr, len, len_u32 })
}

fn read_iov_u32(iov: &[u8; WASI_IOV_SIZE], offset: usize) -> u32 {
    let mut field = [0u8; WASI_U32_SIZE];
    field.copy_from_slice(&iov[offset..offset + WASI_U32_SIZE]);
    u32::from_le_bytes(field)
}

fn checked_fd_write_total(total: u32, len: u32) -> wasmtime::Result<u32> {
    total
        .checked_add(len)
        .ok_or_else(|| wasmtime::Error::msg("fd_write byte count overflow"))
}

fn reserve_fd_capture(
    caller: &mut Caller<'_, HostState>,
    fd: WasiStdioFd,
    len: u32,
) -> wasmtime::Result<()> {
    let len = usize::try_from(len)
        .map_err(|_| wasmtime::Error::msg("fd_write byte count does not fit host usize"))?;
    match fd {
        WasiStdioFd::Stdout if caller.data().config().captures_stdout() => {
            caller.data_mut().reserve_captured_stdout(len)?;
        }
        WasiStdioFd::Stderr if caller.data().config().captures_stderr() => {
            caller.data_mut().reserve_captured_stderr(len)?;
        }
        WasiStdioFd::Stdout | WasiStdioFd::Stderr => {}
    }
    Ok(())
}

fn write_fd_buffer(
    caller: &mut Caller<'_, HostState>,
    memory: &Memory,
    fd: WasiStdioFd,
    ptr: usize,
    len: usize,
) -> wasmtime::Result<()> {
    match fd {
        WasiStdioFd::Stdout if caller.data().config().captures_stdout() => {
            capture_guest_buffer(memory, caller, ptr, len, |state, chunk| {
                state.append_captured_stdout(chunk)
            })?;
        }
        WasiStdioFd::Stdout => write_guest_buffer(memory, caller, ptr, len, std::io::stdout())?,
        WasiStdioFd::Stderr if caller.data().config().captures_stderr() => {
            capture_guest_buffer(memory, caller, ptr, len, |state, chunk| {
                state.append_captured_stderr(chunk)
            })?;
        }
        WasiStdioFd::Stderr => write_guest_buffer(memory, caller, ptr, len, std::io::stderr())?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fd_write_total_rejects_u32_overflow() {
        assert_eq!(checked_fd_write_total(0, 0).unwrap(), 0);
        assert_eq!(checked_fd_write_total(u32::MAX - 1, 1).unwrap(), u32::MAX);
        let err = checked_fd_write_total(u32::MAX, 1)
            .expect_err("fd_write byte count should reject overflow");
        assert!(err.to_string().contains("fd_write byte count overflow"));
    }
}
