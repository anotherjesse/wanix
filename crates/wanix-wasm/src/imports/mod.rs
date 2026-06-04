//! WASI Preview 1 imports linked directly against [`WasiCtx`].
//!
//! Each import reads/writes guest linear memory and delegates the actual
//! syscall to the engine-agnostic [`WasiCtx`], so the same namespace-backed
//! filesystem semantics the QuickJS engine uses apply to any wasm command.
//!
//! This is a focused first-pass subset: enough for a Rust/Go `wasm32-wasi`
//! program doing argv/env, stdio, and file IO. Unsupported calls return `ENOSYS`.

mod fd;
mod mem;
mod path;

use mem::{
    ERRNO_NOSYS, ERRNO_SUCCESS, memory, write_bytes, write_u64, write_vec_buffer, write_vec_sizes,
};
use wanix_wasi::WasiCtx;
use wasmtime::{Caller, Error, Linker, Result};

/// The WASI Preview 1 import module name.
pub(crate) const MODULE: &str = "wasi_snapshot_preview1";

/// Store state for a running WASI command.
pub struct WasiState {
    ctx: WasiCtx,
    clock_ns: u64,
    exit_code: Option<i32>,
}

impl WasiState {
    /// Creates state wrapping a WASI context and a fixed clock value.
    #[must_use]
    pub fn new(ctx: WasiCtx, clock_ns: u64) -> Self {
        Self {
            ctx,
            clock_ns,
            exit_code: None,
        }
    }

    /// Returns the recorded `proc_exit` code, if the guest exited.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

/// Registers the supported `wasi_snapshot_preview1` imports on `linker`.
///
/// # Errors
///
/// Returns an error if a duplicate import name is registered.
pub fn add_to_linker(linker: &mut Linker<WasiState>) -> Result<()> {
    let m = MODULE;

    linker.func_wrap(
        m,
        "proc_exit",
        |mut caller: Caller<'_, WasiState>, code: i32| {
            caller.data_mut().exit_code = Some(code);
            // Unwind the guest; run() recovers the code from exit_code.
            Err::<(), _>(Error::msg(format!("proc_exit({code})")))
        },
    )?;
    linker.func_wrap(m, "sched_yield", |_: Caller<'_, WasiState>| ERRNO_SUCCESS)?;
    linker.func_wrap(
        m,
        "random_get",
        |mut caller: Caller<'_, WasiState>, buf: i32, len: i32| -> Result<i32> {
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
        |mut caller: Caller<'_, WasiState>, _id: i32, _precision: i64, out: i32| -> Result<i32> {
            let now = caller.data().clock_ns;
            let mem = memory(&mut caller)?;
            write_u64(&mem, &mut caller, out, now)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;
    linker.func_wrap(
        m,
        "poll_oneoff",
        |_: Caller<'_, WasiState>, _i: i32, _o: i32, _n: i32, _ne: i32| ERRNO_NOSYS,
    )?;

    // argv / environ: identical two-call (sizes, then buffer) shape.
    linker.func_wrap(
        m,
        "args_sizes_get",
        |mut caller: Caller<'_, WasiState>, count: i32, size: i32| -> Result<i32> {
            let v = caller.data().ctx.args().to_vec();
            write_vec_sizes(&mut caller, &v, count, size)
        },
    )?;
    linker.func_wrap(
        m,
        "args_get",
        |mut caller: Caller<'_, WasiState>, ptrs: i32, buf: i32| -> Result<i32> {
            let v = caller.data().ctx.args().to_vec();
            write_vec_buffer(&mut caller, &v, ptrs, buf)
        },
    )?;
    linker.func_wrap(
        m,
        "environ_sizes_get",
        |mut caller: Caller<'_, WasiState>, count: i32, size: i32| -> Result<i32> {
            let v = caller.data().ctx.env().to_vec();
            write_vec_sizes(&mut caller, &v, count, size)
        },
    )?;
    linker.func_wrap(
        m,
        "environ_get",
        |mut caller: Caller<'_, WasiState>, ptrs: i32, buf: i32| -> Result<i32> {
            let v = caller.data().ctx.env().to_vec();
            write_vec_buffer(&mut caller, &v, ptrs, buf)
        },
    )?;

    fd::register(linker)?;
    path::register(linker)?;
    Ok(())
}
