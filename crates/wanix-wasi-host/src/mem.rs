//! Guest linear-memory access and WASI Preview 1 encoding helpers.
//!
//! These are generic over the store state `S`: they only touch guest memory and
//! plain values, never the host context, so they work for any [`WasiHost`].

use wanix_wasi::Errno;
use wasmtime::{Caller, Error, Extern, Memory, Result};

use super::ERRNO_SUCCESS;

/// Returns the guest's exported linear memory.
pub(super) fn memory<S>(caller: &mut Caller<'_, S>) -> Result<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(mem)) => Ok(mem),
        _ => Err(Error::msg("wasm module does not export 'memory'")),
    }
}

pub(super) fn read_bytes<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    ptr: i32,
    len: usize,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    mem.read(caller, ptr as usize, &mut buf)
        .map_err(|e| Error::msg(format!("guest memory read: {e}")))?;
    Ok(buf)
}

pub(super) fn read_str<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    ptr: i32,
    len: i32,
) -> Result<String> {
    let bytes = read_bytes(mem, caller, ptr, len.max(0) as usize)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn write_bytes<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    ptr: i32,
    bytes: &[u8],
) -> Result<()> {
    mem.write(caller, ptr as usize, bytes)
        .map_err(|e| Error::msg(format!("guest memory write: {e}")))
}

pub(super) fn write_u32<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    ptr: i32,
    value: u32,
) -> Result<()> {
    write_bytes(mem, caller, ptr, &value.to_le_bytes())
}

pub(super) fn write_u64<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    ptr: i32,
    value: u64,
) -> Result<()> {
    write_bytes(mem, caller, ptr, &value.to_le_bytes())
}

pub(super) fn read_u32<S>(mem: &Memory, caller: &mut Caller<'_, S>, ptr: i32) -> Result<u32> {
    let bytes = read_bytes(mem, caller, ptr, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
}

/// Reads an array of WASI `iovec` (ptr,len) pairs.
pub(super) fn read_iovs<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    iovs: i32,
    iovs_len: i32,
) -> Result<Vec<(i32, usize)>> {
    let mut out = Vec::with_capacity(iovs_len.max(0) as usize);
    for i in 0..iovs_len.max(0) {
        let base = iovs + i * 8;
        let ptr = read_u32(mem, caller, base)? as i32;
        let len = read_u32(mem, caller, base + 4)? as usize;
        out.push((ptr, len));
    }
    Ok(out)
}

/// Writes the (count, total-buffer-size) pair for an argv/environ vector.
pub(super) fn write_vec_sizes<S>(
    caller: &mut Caller<'_, S>,
    v: &[String],
    count: i32,
    size: i32,
) -> Result<i32> {
    let total: usize = v.iter().map(|s| s.len() + 1).sum();
    let mem = memory(caller)?;
    write_u32(&mem, caller, count, v.len() as u32)?;
    write_u32(&mem, caller, size, total as u32)?;
    Ok(ERRNO_SUCCESS)
}

/// Writes the pointer table and null-terminated buffer for an argv/environ vector.
pub(super) fn write_vec_buffer<S>(
    caller: &mut Caller<'_, S>,
    v: &[String],
    ptrs: i32,
    buf: i32,
) -> Result<i32> {
    let mem = memory(caller)?;
    let mut cursor = buf;
    for (i, s) in v.iter().enumerate() {
        write_u32(&mem, caller, ptrs + (i as i32) * 4, cursor as u32)?;
        let mut bytes = s.clone().into_bytes();
        bytes.push(0);
        write_bytes(&mem, caller, cursor, &bytes)?;
        cursor += bytes.len() as i32;
    }
    Ok(ERRNO_SUCCESS)
}

/// Maps a `Result<(), Errno>` to a Preview 1 errno return.
pub(super) fn code(r: core::result::Result<(), Errno>) -> i32 {
    match r {
        Ok(()) => ERRNO_SUCCESS,
        Err(e) => e.preview1_code() as i32,
    }
}

/// Writes a partial byte count then returns the errno (for fd_read/fd_write).
pub(super) fn errno<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    nout: i32,
    partial: usize,
    e: Errno,
) -> Result<i32> {
    write_u32(mem, caller, nout, partial as u32)?;
    Ok(e.preview1_code() as i32)
}
