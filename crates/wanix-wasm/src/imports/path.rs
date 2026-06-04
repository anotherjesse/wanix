//! WASI Preview 1 `path_*` imports.

use super::WasiState;
use super::mem::{ERRNO_SUCCESS, code, memory, read_str, write_bytes, write_u32};
use wanix_wasi::{WasiFd, WasiRights};
use wasmtime::{Caller, Linker, Result};

/// Registers the `path_*` imports on `linker`.
pub(super) fn register(linker: &mut Linker<WasiState>) -> Result<()> {
    let m = super::MODULE;

    linker.func_wrap(
        m,
        "path_open",
        |mut caller: Caller<'_, WasiState>,
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
            let result = caller.data_mut().ctx.path_open_preview1(
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
        |mut caller: Caller<'_, WasiState>,
         dirfd: i32,
         flags: i32,
         path: i32,
         path_len: i32,
         out: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            match caller.data().ctx.path_filestat_get_with_flags(
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
        "path_create_directory",
        |mut caller: Caller<'_, WasiState>, dirfd: i32, path: i32, path_len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            Ok(code(
                caller
                    .data()
                    .ctx
                    .path_create_directory(WasiFd::new(dirfd as u32), &name),
            ))
        },
    )?;
    linker.func_wrap(
        m,
        "path_unlink_file",
        |mut caller: Caller<'_, WasiState>, dirfd: i32, path: i32, path_len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            Ok(code(
                caller
                    .data()
                    .ctx
                    .path_unlink_file(WasiFd::new(dirfd as u32), &name),
            ))
        },
    )?;
    linker.func_wrap(
        m,
        "path_remove_directory",
        |mut caller: Caller<'_, WasiState>, dirfd: i32, path: i32, path_len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            Ok(code(
                caller
                    .data()
                    .ctx
                    .path_remove_directory(WasiFd::new(dirfd as u32), &name),
            ))
        },
    )?;
    linker.func_wrap(
        m,
        "path_rename",
        |mut caller: Caller<'_, WasiState>,
         old_fd: i32,
         old_path: i32,
         old_path_len: i32,
         new_fd: i32,
         new_path: i32,
         new_path_len: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let old_name = read_str(&mem, &mut caller, old_path, old_path_len)?;
            let new_name = read_str(&mem, &mut caller, new_path, new_path_len)?;
            Ok(code(caller.data().ctx.path_rename(
                WasiFd::new(old_fd as u32),
                &old_name,
                WasiFd::new(new_fd as u32),
                &new_name,
            )))
        },
    )?;
    Ok(())
}
