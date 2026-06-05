//! WASI Preview 1 `fd_*` imports, generic over any [`WasiHost`].

use wanix_wasi::{WasiFd, WasiWhence};
use wasmtime::{Caller, Linker, Result};

use super::WasiHost;
use super::mem::{code, memory, write_bytes, write_u32, write_u64};
use super::{ERRNO_INVAL, ERRNO_SUCCESS};

mod dirent;
mod io;

use dirent::write_direntries;

/// Registers the `fd_*` imports on `linker`.
pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::MODULE;

    io::register(linker)?;
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
            match caller
                .data_mut()
                .wasi()
                .fd_fdstat_get(WasiFd::new(fd as u32))
            {
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
            match caller
                .data_mut()
                .wasi()
                .fd_prestat_get(WasiFd::new(fd as u32))
            {
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
        "fd_filestat_set_times",
        |mut caller: Caller<'_, S>, fd: i32, atim: i64, mtim: i64, fst_flags: i32| {
            code(caller.data_mut().wasi().fd_filestat_set_times(
                WasiFd::new(fd as u32),
                atim as u64,
                mtim as u64,
                fst_flags as u16,
            ))
        },
    )?;
    linker.func_wrap(
        m,
        "fd_filestat_get",
        |mut caller: Caller<'_, S>, fd: i32, out: i32| -> Result<i32> {
            match caller
                .data_mut()
                .wasi()
                .fd_filestat_get(WasiFd::new(fd as u32))
            {
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
