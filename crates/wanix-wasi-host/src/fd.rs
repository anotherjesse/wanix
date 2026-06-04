//! WASI Preview 1 `fd_*` imports, generic over any [`WasiHost`].

use wanix_fs::DirEntry;
use wanix_wasi::{WasiFd, WasiFileType, WasiWhence};
use wasmtime::{Caller, Linker, Memory, Result};

use super::WasiHost;
use super::mem::{
    code, errno, memory, read_bytes, read_iovs, write_bytes, write_u32, write_u64,
};
use super::{ERRNO_INVAL, ERRNO_SUCCESS};

/// Byte size of a WASI Preview 1 `dirent` header.
const DIRENT_SIZE: usize = 24;
const DIRENT_NEXT_OFFSET: usize = 0;
const DIRENT_INO_OFFSET: usize = 8;
const DIRENT_NAMLEN_OFFSET: usize = 16;
const DIRENT_FILETYPE_OFFSET: usize = 20;

/// Registers the `fd_*` imports on `linker`.
pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::MODULE;

    linker.func_wrap(
        m,
        "fd_write",
        |mut caller: Caller<'_, S>,
         fd: i32,
         iovs: i32,
         iovs_len: i32,
         nout: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let mut written = 0usize;
            for (ptr, len) in read_iovs(&mem, &mut caller, iovs, iovs_len)? {
                let buf = read_bytes(&mem, &mut caller, ptr, len)?;
                match caller.data_mut().wasi().fd_write(WasiFd::new(fd as u32), &buf) {
                    Ok(n) => {
                        written += n;
                        if n < buf.len() {
                            break;
                        }
                    }
                    Err(e) => return errno(&mem, &mut caller, nout, written, e),
                }
            }
            let mem = memory(&mut caller)?;
            write_u32(&mem, &mut caller, nout, written as u32)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    linker.func_wrap(
        m,
        "fd_read",
        |mut caller: Caller<'_, S>,
         fd: i32,
         iovs: i32,
         iovs_len: i32,
         nout: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let mut total = 0usize;
            for (ptr, len) in read_iovs(&mem, &mut caller, iovs, iovs_len)? {
                let mut buf = vec![0u8; len];
                match caller
                    .data_mut()
                    .wasi()
                    .fd_read(WasiFd::new(fd as u32), &mut buf)
                {
                    Ok(n) => {
                        write_bytes(&mem, &mut caller, ptr, &buf[..n])?;
                        total += n;
                        if n < len {
                            break;
                        }
                    }
                    Err(e) => return errno(&mem, &mut caller, nout, total, e),
                }
            }
            let mem = memory(&mut caller)?;
            write_u32(&mem, &mut caller, nout, total as u32)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    linker.func_wrap(m, "fd_close", |mut caller: Caller<'_, S>, fd: i32| {
        code(caller.data_mut().wasi().fd_close(WasiFd::new(fd as u32)))
    })?;
    linker.func_wrap(
        m,
        "fd_seek",
        |mut caller: Caller<'_, S>, fd: i32, offset: i64, whence: i32, out: i32| -> Result<i32> {
            let w = match WasiWhence::from_preview1(whence) {
                Ok(w) => w,
                Err(_) => return Ok(ERRNO_INVAL),
            };
            match caller
                .data_mut()
                .wasi()
                .fd_seek(WasiFd::new(fd as u32), offset, w)
            {
                Ok(pos) => {
                    let mem = memory(&mut caller)?;
                    write_u64(&mem, &mut caller, out, pos)?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    linker.func_wrap(
        m,
        "fd_tell",
        |mut caller: Caller<'_, S>, fd: i32, out: i32| -> Result<i32> {
            match caller.data_mut().wasi().fd_tell(WasiFd::new(fd as u32)) {
                Ok(pos) => {
                    let mem = memory(&mut caller)?;
                    write_u64(&mem, &mut caller, out, pos)?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    linker.func_wrap(
        m,
        "fd_fdstat_get",
        |mut caller: Caller<'_, S>, fd: i32, out: i32| -> Result<i32> {
            match caller.data_mut().wasi().fd_fdstat_get(WasiFd::new(fd as u32)) {
                Ok(stat) => {
                    let bytes = stat.to_preview1_bytes();
                    let mem = memory(&mut caller)?;
                    write_bytes(&mem, &mut caller, out, &bytes)?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    linker.func_wrap(
        m,
        "fd_fdstat_set_flags",
        |mut caller: Caller<'_, S>, fd: i32, flags: i32| {
            code(
                caller
                    .data_mut()
                    .wasi()
                    .fd_fdstat_set_flags(WasiFd::new(fd as u32), flags as u16),
            )
        },
    )?;
    linker.func_wrap(
        m,
        "fd_prestat_get",
        |mut caller: Caller<'_, S>, fd: i32, out: i32| -> Result<i32> {
            match caller.data_mut().wasi().fd_prestat_get(WasiFd::new(fd as u32)) {
                Ok(prestat) => {
                    let bytes = prestat.to_preview1_bytes();
                    let mem = memory(&mut caller)?;
                    write_bytes(&mem, &mut caller, out, &bytes)?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    linker.func_wrap(
        m,
        "fd_prestat_dir_name",
        |mut caller: Caller<'_, S>, fd: i32, path: i32, path_len: i32| -> Result<i32> {
            let mut buf = vec![0u8; path_len.max(0) as usize];
            match caller
                .data_mut()
                .wasi()
                .fd_prestat_dir_name(WasiFd::new(fd as u32), &mut buf)
            {
                Ok(n) => {
                    let mem = memory(&mut caller)?;
                    write_bytes(&mem, &mut caller, path, &buf[..n])?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    linker.func_wrap(
        m,
        "fd_filestat_set_size",
        |mut caller: Caller<'_, S>, fd: i32, size: i64| {
            code(
                caller
                    .data_mut()
                    .wasi()
                    .fd_filestat_set_size(WasiFd::new(fd as u32), size as u64),
            )
        },
    )?;
    linker.func_wrap(
        m,
        "fd_filestat_get",
        |mut caller: Caller<'_, S>, fd: i32, out: i32| -> Result<i32> {
            match caller.data_mut().wasi().fd_filestat_get(WasiFd::new(fd as u32)) {
                Ok(stat) => {
                    let bytes = stat.to_preview1_bytes();
                    let mem = memory(&mut caller)?;
                    write_bytes(&mem, &mut caller, out, &bytes)?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    linker.func_wrap(
        m,
        "fd_readdir",
        |mut caller: Caller<'_, S>,
         fd: i32,
         buf: i32,
         buf_len: i32,
         cookie: i64,
         bufused: i32|
         -> Result<i32> {
            let entries = match caller.data_mut().wasi().fd_read_dir(WasiFd::new(fd as u32)) {
                Ok(entries) => entries,
                Err(e) => return Ok(e.preview1_code() as i32),
            };
            let mem = memory(&mut caller)?;
            let used = write_direntries(
                &mem,
                &mut caller,
                buf,
                buf_len.max(0) as usize,
                cookie as usize,
                &entries,
            )?;
            write_u32(&mem, &mut caller, bufused, used as u32)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    Ok(())
}

/// Encodes Preview 1 dirents into guest memory, returning the bytes written.
///
/// Mirrors the WASI wire encoding: a fixed [`DIRENT_SIZE`] header followed by
/// the raw name bytes per entry. A truncated entry (header or name clipped by
/// the buffer) is the final entry written, matching `fd_readdir` semantics.
fn write_direntries<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    buf: i32,
    buf_len: usize,
    cookie: usize,
    entries: &[DirEntry],
) -> Result<usize> {
    let mut used = 0usize;
    for (index, entry) in entries.iter().enumerate().skip(cookie) {
        let remaining = buf_len - used;
        if remaining == 0 {
            break;
        }
        let name = entry.name().as_bytes();
        let entry_len = DIRENT_SIZE + name.len();
        let to_write = remaining.min(entry_len);
        let header = dirent_header(index, entry);
        let header_len = to_write.min(DIRENT_SIZE);
        write_bytes(mem, caller, buf + used as i32, &header[..header_len])?;
        if to_write > DIRENT_SIZE {
            let name_len = to_write - DIRENT_SIZE;
            write_bytes(mem, caller, buf + (used + DIRENT_SIZE) as i32, &name[..name_len])?;
        }
        used += to_write;
        if to_write < entry_len {
            break;
        }
    }
    Ok(used)
}

/// Builds the fixed-size Preview 1 dirent header for `entry` at `index`.
fn dirent_header(index: usize, entry: &DirEntry) -> [u8; DIRENT_SIZE] {
    let next = (index as u64) + 1;
    let name_len = entry.name().len() as u32;
    let file_type = WasiFileType::from_file_type(entry.metadata().file_type()).preview1_code();
    let mut bytes = [0u8; DIRENT_SIZE];
    bytes[DIRENT_NEXT_OFFSET..DIRENT_NEXT_OFFSET + 8].copy_from_slice(&next.to_le_bytes());
    bytes[DIRENT_INO_OFFSET..DIRENT_INO_OFFSET + 8].copy_from_slice(&0u64.to_le_bytes());
    bytes[DIRENT_NAMLEN_OFFSET..DIRENT_NAMLEN_OFFSET + 4].copy_from_slice(&name_len.to_le_bytes());
    bytes[DIRENT_FILETYPE_OFFSET] = file_type;
    bytes
}
