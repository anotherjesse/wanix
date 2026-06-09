//! The generic command-style WASI Preview 1 Wasmtime linker over [`WasiCtx`].
//!
//! This crate is the guest-memory marshalling that turns Preview 1 imports into
//! [`WasiCtx`] calls for an arbitrary `wasm32-wasi` command module. The
//! `wanix-wasm` runner uses it directly; the QuickJS engine does **not** — qjs
//! keeps its own engine-local host-import path (with snapshot blockers, live fd
//! readiness, restore reattachment, and a richer `poll_oneoff`), sharing only the
//! `wanix-wasi` [`WasiCtx`] and task-config contracts, not this linker.
//!
//! A host store-state implements [`WasiHost`] to expose its [`WasiCtx`], clock,
//! and exit hook; what differs per consumer (the wasm module, the command entry)
//! stays in the consumer. Fixing a Preview 1 detail here fixes it for every
//! consumer of this linker.
//!
//! ## Blocking reads and the `poll_oneoff` subset
//!
//! This linker gives command tasks POSIX-style *blocking* reads (ADR 0010
//! tier 2): `fd_read` on an fd that reports not-ready (a queue-backed device
//! like `#term/<id>/program` or `#pipe` with nothing buffered) parks the host
//! thread with bounded-backoff readiness polling until data arrives, the
//! device errors, or end-of-stream is decidable. Regular byte files always
//! report ready — including at EOF — so they never wait.
//!
//! `poll_oneoff` supports exactly one subscription kind: `fd_read` readiness.
//! It blocks until at least one subscribed fd is ready (or its readiness
//! probe fails, which is reported as that event's errno) and returns the
//! decided events. Clock subscriptions, `fd_write` subscriptions, and unknown
//! tags deliberately return `ERRNO_NOSYS` — the host stays a command-style
//! WASI subset and refuses what it does not implement.

use wanix_wasi::WasiCtx;
use wasmtime::{Linker, Result};

mod core;
mod fd;
mod mem;
mod path;
mod poll;
mod process;
mod vectors;
mod wait;

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

/// Registers the supported `wasi_snapshot_preview1` imports on `linker`.
///
/// # Errors
///
/// Returns an error if a duplicate import name is registered (unless the linker
/// allows shadowing).
pub fn add_to_linker<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    process::register(linker)?;
    core::register(linker)?;
    poll::register(linker)?;
    vectors::register(linker)?;
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
    fn add_to_linker_registers_preview1_import_families() {
        let engine = Engine::default();
        let mut linker = Linker::<TestHost>::new(&engine);
        add_to_linker(&mut linker).expect("imports register");

        let mut store = Store::new(&engine, TestHost::new());
        for name in [
            "proc_exit",
            "sched_yield",
            "random_get",
            "clock_time_get",
            "poll_oneoff",
            "args_sizes_get",
            "args_get",
            "environ_sizes_get",
            "environ_get",
            "fd_read",
            "fd_write",
            "fd_close",
            "fd_seek",
            "fd_tell",
            "fd_fdstat_get",
            "fd_fdstat_set_flags",
            "fd_prestat_get",
            "fd_prestat_dir_name",
            "fd_filestat_set_size",
            "fd_filestat_set_times",
            "fd_filestat_get",
            "fd_readdir",
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
                "missing WASI Preview 1 import {name}"
            );
        }
    }
}
