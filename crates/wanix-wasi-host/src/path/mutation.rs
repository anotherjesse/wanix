//! Mutating WASI Preview 1 `path_*` imports.

use wanix_wasi::WasiFd;
use wasmtime::{Caller, Linker, Result};

use super::super::WasiHost;
use super::super::mem::{code, memory, read_bytes, read_str};

pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    register_filestat_set_times(linker)?;
    register_create_directory(linker)?;
    register_remove_directory(linker)?;
    register_unlink_file(linker)?;
    register_rename(linker)?;
    register_symlink(linker)?;
    Ok(())
}

fn register_filestat_set_times<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::super::MODULE;

    linker.func_wrap(
        m,
        "path_filestat_set_times",
        |mut caller: Caller<'_, S>,
         dirfd: i32,
         flags: i32,
         path: i32,
         path_len: i32,
         atim: i64,
         mtim: i64,
         fst_flags: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            Ok(code(caller.data_mut().wasi().path_filestat_set_times(
                WasiFd::new(dirfd as u32),
                flags as u32,
                &name,
                atim as u64,
                mtim as u64,
                fst_flags as u16,
            )))
        },
    )?;
    Ok(())
}

fn register_create_directory<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::super::MODULE;

    linker.func_wrap(
        m,
        "path_create_directory",
        |mut caller: Caller<'_, S>, dirfd: i32, path: i32, path_len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            Ok(code(
                caller
                    .data_mut()
                    .wasi()
                    .path_create_directory(WasiFd::new(dirfd as u32), &name),
            ))
        },
    )?;
    Ok(())
}

fn register_remove_directory<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::super::MODULE;

    linker.func_wrap(
        m,
        "path_remove_directory",
        |mut caller: Caller<'_, S>, dirfd: i32, path: i32, path_len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            Ok(code(
                caller
                    .data_mut()
                    .wasi()
                    .path_remove_directory(WasiFd::new(dirfd as u32), &name),
            ))
        },
    )?;
    Ok(())
}

fn register_unlink_file<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::super::MODULE;

    linker.func_wrap(
        m,
        "path_unlink_file",
        |mut caller: Caller<'_, S>, dirfd: i32, path: i32, path_len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            let name = read_str(&mem, &mut caller, path, path_len)?;
            Ok(code(
                caller
                    .data_mut()
                    .wasi()
                    .path_unlink_file(WasiFd::new(dirfd as u32), &name),
            ))
        },
    )?;
    Ok(())
}

fn register_rename<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::super::MODULE;

    linker.func_wrap(
        m,
        "path_rename",
        |mut caller: Caller<'_, S>,
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
            Ok(code(caller.data_mut().wasi().path_rename(
                WasiFd::new(old_fd as u32),
                &old_name,
                WasiFd::new(new_fd as u32),
                &new_name,
            )))
        },
    )?;
    Ok(())
}

fn register_symlink<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::super::MODULE;

    linker.func_wrap(
        m,
        "path_symlink",
        // WASI ABI: the link target (old_path) precedes `dirfd`.
        |mut caller: Caller<'_, S>,
         old_path: i32,
         old_path_len: i32,
         dirfd: i32,
         new_path: i32,
         new_path_len: i32|
         -> Result<i32> {
            let mem = memory(&mut caller)?;
            // Targets need not be UTF-8, so read raw bytes (matching qjs).
            let target = read_bytes(&mem, &mut caller, old_path, old_path_len.max(0) as usize)?;
            let new_name = read_str(&mem, &mut caller, new_path, new_path_len)?;
            Ok(code(caller.data_mut().wasi().path_symlink(
                &target,
                WasiFd::new(dirfd as u32),
                &new_name,
            )))
        },
    )?;
    Ok(())
}
