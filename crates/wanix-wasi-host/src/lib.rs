//! The single WASI Preview 1 Wasmtime linker over [`WasiCtx`].
//!
//! Both the QuickJS engine and the generic `wanix-wasm` task runner are "a wasm
//! module on Wasmtime backed by Wanix WASI". This crate is the shared half: the
//! guest-memory marshalling that turns Preview 1 imports into [`WasiCtx`] calls.
//! A host store-state implements [`WasiHost`] to expose its [`WasiCtx`], clock,
//! and exit hook; everything that differs between task types (the wasm module,
//! the QuickJS `env` imports, the reactor-vs-command entry) stays in the
//! consumer. Fixing a Preview 1 detail here fixes it for every task type.

use wanix_wasi::WasiCtx;
use wasmtime::{Linker, Result};

mod fd;
mod mem;
mod path;

/// A Wasmtime store state that can back WASI Preview 1 imports.
///
/// Implementors expose their [`WasiCtx`], a deterministic clock, and a hook the
/// linker calls on `proc_exit` (after which the guest is unwound via a trap).
pub trait WasiHost: Send {
    /// Returns the WASI context whose namespace backs every syscall.
    fn wasi(&mut self) -> &mut WasiCtx;

    /// Returns the nanosecond clock value reported by `clock_time_get`.
    fn clock_time_ns(&self) -> u64;

    /// Records the exit code passed to `proc_exit`.
    fn on_proc_exit(&mut self, code: i32);
}

pub(crate) const MODULE: &str = "wasi_snapshot_preview1";
pub(crate) const ERRNO_SUCCESS: i32 = 0;
pub(crate) const ERRNO_INVAL: i32 = 28;
pub(crate) const ERRNO_NOSYS: i32 = 52;

use mem::{memory, write_bytes, write_u64, write_vec_buffer, write_vec_sizes};
use wasmtime::{Caller, Error};

/// Registers the supported `wasi_snapshot_preview1` imports on `linker`.
///
/// # Errors
///
/// Returns an error if a duplicate import name is registered (unless the linker
/// allows shadowing).
pub fn add_to_linker<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = MODULE;

    linker.func_wrap(m, "proc_exit", |mut caller: Caller<'_, S>, code: i32| {
        caller.data_mut().on_proc_exit(code);
        // Unwind the guest; the consumer recovers the code from its exit hook.
        Err::<(), _>(Error::msg(format!("proc_exit({code})")))
    })?;
    linker.func_wrap(m, "sched_yield", |_: Caller<'_, S>| ERRNO_SUCCESS)?;
    linker.func_wrap(
        m,
        "random_get",
        |mut caller: Caller<'_, S>, buf: i32, len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            // Deterministic fill; this runner targets reproducible tasks.
            let zeros = vec![7u8; len.max(0) as usize];
            write_bytes(&mem, &mut caller, buf, &zeros)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    linker.func_wrap(
        m,
        "clock_time_get",
        |mut caller: Caller<'_, S>, _id: i32, _precision: i64, out: i32| -> Result<i32> {
            let now = caller.data().clock_time_ns();
            let mem = memory(&mut caller)?;
            write_u64(&mem, &mut caller, out, now)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    linker.func_wrap(
        m,
        "poll_oneoff",
        |_: Caller<'_, S>, _i: i32, _o: i32, _n: i32, _ne: i32| ERRNO_NOSYS,
    )?;

    // argv / environ: identical two-call (sizes, then buffer) shape.
    linker.func_wrap(
        m,
        "args_sizes_get",
        |mut caller: Caller<'_, S>, count: i32, size: i32| -> Result<i32> {
            let v = caller.data_mut().wasi().args().to_vec();
            write_vec_sizes(&mut caller, &v, count, size)
        },
    )?;
    linker.func_wrap(
        m,
        "args_get",
        |mut caller: Caller<'_, S>, ptrs: i32, buf: i32| -> Result<i32> {
            let v = caller.data_mut().wasi().args().to_vec();
            write_vec_buffer(&mut caller, &v, ptrs, buf)
        },
    )?;
    linker.func_wrap(
        m,
        "environ_sizes_get",
        |mut caller: Caller<'_, S>, count: i32, size: i32| -> Result<i32> {
            let v = caller.data_mut().wasi().env().to_vec();
            write_vec_sizes(&mut caller, &v, count, size)
        },
    )?;
    linker.func_wrap(
        m,
        "environ_get",
        |mut caller: Caller<'_, S>, ptrs: i32, buf: i32| -> Result<i32> {
            let v = caller.data_mut().wasi().env().to_vec();
            write_vec_buffer(&mut caller, &v, ptrs, buf)
        },
    )?;

    fd::register(linker)?;
    path::register(linker)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use wanix_wasi::{WasiConfig, WasiCtx};
    use wasmtime::{Engine, Linker, Store};

    use super::{MODULE, WasiHost, add_to_linker};

    struct TestHost {
        wasi: WasiCtx,
        exit_code: Option<i32>,
    }

    impl TestHost {
        fn new() -> Self {
            Self {
                wasi: WasiCtx::new(WasiConfig::new(Default::default())),
                exit_code: None,
            }
        }
    }

    impl WasiHost for TestHost {
        fn wasi(&mut self) -> &mut WasiCtx {
            &mut self.wasi
        }

        fn clock_time_ns(&self) -> u64 {
            0
        }

        fn on_proc_exit(&mut self, code: i32) {
            self.exit_code = Some(code);
        }
    }

    #[test]
    fn add_to_linker_registers_path_imports() {
        let engine = Engine::default();
        let mut linker = Linker::<TestHost>::new(&engine);
        add_to_linker(&mut linker).expect("imports register");

        let mut store = Store::new(&engine, TestHost::new());
        for name in [
            "path_open",
            "path_filestat_get",
            "path_readlink",
            "path_filestat_set_times",
            "path_create_directory",
            "path_remove_directory",
            "path_unlink_file",
            "path_rename",
            "path_symlink",
        ] {
            assert!(
                linker.get(&mut store, MODULE, name).is_ok(),
                "missing WASI path import {name}"
            );
        }
    }
}
