//! WASI Preview 1 `path_*` imports, generic over any [`WasiHost`].

use wanix_wasi::{WasiFd, WasiRights};
use wasmtime::{Caller, Linker, Result};

use super::ERRNO_SUCCESS;
use super::WasiHost;
use super::mem::{memory, read_str, write_bytes, write_u32};

mod mutation;

/// Registers the `path_*` imports on `linker`.
pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::MODULE;

    linker.func_wrap(
        m,
        "path_open",
        |mut caller: Caller<'_, S>,
         dirfd: i32,
         _dirflags: i32,
         path: i32,
         path_len: i32,
         oflags: i32,
         rights_base: i64,
         rights_inheriting: i64,
         fdflags: i32,
         out: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            let result = caller.data_mut().wasi().path_open_preview1(
                WasiFd::new(dirfd as u32),
                &name,
                oflags as u16,
                WasiRights::from_preview1_bits(rights_base as u64),
                WasiRights::from_preview1_bits(rights_inheriting as u64),
                fdflags as u16,
            );
            match result {
                Ok(fd) => {
                    let mem = memory(&mut caller)?;
                    write_u32(&mem, &mut caller, out, fd.get())?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    linker.func_wrap(
        m,
        "path_filestat_get",
        |mut caller: Caller<'_, S>,
         dirfd: i32,
         flags: i32,
         path: i32,
         path_len: i32,
         out: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            match caller.data_mut().wasi().path_filestat_get_with_flags(
                WasiFd::new(dirfd as u32),
                flags as u32,
                &name,
            ) {
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
        "path_readlink",
        |mut caller: Caller<'_, S>,
         dirfd: i32,
         path: i32,
         path_len: i32,
         buf: i32,
         buf_len: i32,
         bufused: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            match caller
                .data_mut()
                .wasi()
                .path_readlink(WasiFd::new(dirfd as u32), &name)
            {
                Ok(target) => {
                    let count = target.len().min(buf_len.max(0) as usize);
                    let mem = memory(&mut caller)?;
                    write_bytes(&mem, &mut caller, buf, &target[..count])?;
                    write_u32(&mem, &mut caller, bufused, count as u32)?;
                    Ok(ERRNO_SUCCESS)
                }
                Err(e) => Ok(e.preview1_code() as i32),
            }
        },
    )?;
    mutation::register(linker)?;
    Ok(())
}
