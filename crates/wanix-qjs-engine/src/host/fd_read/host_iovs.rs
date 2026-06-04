use super::super::{HostState, QuickJsWasiErrno, QuickJsWasiHost};
use super::ReadRequest;
use super::iov::{GuestIov, checked_fd_read_total, fd_read_count, read_valid_iov};
use wasmtime::{Caller, Memory};

enum ReadIovOutcome {
    Continue(u32),
    Stop(u32),
}

enum HostReadLoop {
    Continue(u32),
    Stop(u32),
    Errno(QuickJsWasiErrno),
}

pub(super) fn read_host_iovs(
    host: &mut dyn QuickJsWasiHost,
    fd: u32,
    request: &ReadRequest,
    caller: &mut Caller<'_, HostState>,
    iovs_ptr: i32,
) -> wasmtime::Result<Result<u32, QuickJsWasiErrno>> {
    let mut total_read = 0u32;
    for index in 0..request.iovs_len {
        match advance_host_read(host, fd, request, caller, iovs_ptr, index, total_read)? {
            HostReadLoop::Continue(next_total) => total_read = next_total,
            HostReadLoop::Stop(final_total) => return Ok(Ok(final_total)),
            HostReadLoop::Errno(errno) => return Ok(Err(errno)),
        }
    }
    Ok(Ok(total_read))
}

fn advance_host_read(
    host: &mut dyn QuickJsWasiHost,
    fd: u32,
    request: &ReadRequest,
    caller: &mut Caller<'_, HostState>,
    iovs_ptr: i32,
    index: usize,
    total_read: u32,
) -> wasmtime::Result<HostReadLoop> {
    let outcome = read_host_iov_at(host, fd, request, caller, iovs_ptr, index)?;
    host_read_loop_outcome(outcome, total_read)
}

fn host_read_loop_outcome(
    outcome: Result<ReadIovOutcome, QuickJsWasiErrno>,
    total_read: u32,
) -> wasmtime::Result<HostReadLoop> {
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(errno) => return Ok(HostReadLoop::Errno(errno)),
    };
    advance_host_read_total(outcome, total_read)
}

fn advance_host_read_total(
    outcome: ReadIovOutcome,
    total_read: u32,
) -> wasmtime::Result<HostReadLoop> {
    match outcome {
        ReadIovOutcome::Continue(count) => Ok(HostReadLoop::Continue(checked_fd_read_total(
            total_read, count,
        )?)),
        ReadIovOutcome::Stop(count) => Ok(HostReadLoop::Stop(checked_fd_read_total(
            total_read, count,
        )?)),
    }
}

fn read_host_iov_at(
    host: &mut dyn QuickJsWasiHost,
    fd: u32,
    request: &ReadRequest,
    caller: &mut Caller<'_, HostState>,
    iovs_ptr: i32,
    index: usize,
) -> wasmtime::Result<Result<ReadIovOutcome, QuickJsWasiErrno>> {
    let iov = read_valid_iov(&request.memory, &*caller, iovs_ptr, index)?;
    let Some(iov) = non_empty_iov(iov) else {
        return Ok(Ok(ReadIovOutcome::Continue(0)));
    };
    let count = match read_host_iov(host, fd, &request.memory, caller, &iov)? {
        Ok(count) => count,
        Err(errno) => return Ok(Err(errno)),
    };
    Ok(Ok(host_read_iov_outcome(fd_read_count(count)?, iov.len)))
}

fn non_empty_iov(iov: GuestIov) -> Option<GuestIov> {
    if iov.len == 0 { None } else { Some(iov) }
}

fn host_read_iov_outcome(count: u32, iov_len: usize) -> ReadIovOutcome {
    if count as usize == iov_len {
        ReadIovOutcome::Continue(count)
    } else {
        ReadIovOutcome::Stop(count)
    }
}

fn read_host_iov(
    host: &mut dyn QuickJsWasiHost,
    fd: u32,
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    iov: &GuestIov,
) -> wasmtime::Result<Result<usize, QuickJsWasiErrno>> {
    let mut bytes = host_read_buffer(iov.len)?;
    let count = match host_read_count(host, fd, &mut bytes) {
        Ok(count) => count,
        Err(errno) => return Ok(Err(errno)),
    };
    validate_host_read_count(count, iov.len)?;
    memory.write(caller, iov.ptr, &bytes[..count])?;
    Ok(Ok(count))
}

fn host_read_buffer(len: usize) -> wasmtime::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| wasmtime::Error::msg("fd_read buffer allocation failed"))?;
    bytes.resize(len, 0);
    Ok(bytes)
}

fn host_read_count(
    host: &mut dyn QuickJsWasiHost,
    fd: u32,
    bytes: &mut [u8],
) -> Result<usize, QuickJsWasiErrno> {
    host.fd_read(fd, bytes)
}

fn validate_host_read_count(count: usize, iov_len: usize) -> wasmtime::Result<()> {
    if count > iov_len {
        return Err(wasmtime::Error::msg(
            "QuickJS WASI host returned oversized fd_read count",
        ));
    }
    Ok(())
}
