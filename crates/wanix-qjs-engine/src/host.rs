use crate::guest::guest_offset;
use anyhow::Result;
use guest_memory::{fill_guest_buffer, guest_len};
use wasmtime::{Caller, Linker, Memory};

mod call;
mod callback;
mod config;
mod fd_write;
mod fs;
mod guest_memory;
mod module_loader;
mod promise_rejection;
mod state;
mod wasi_host;

pub(crate) use callback::{HostCallbackEntry, HostCallbackMode, scalar_host_callback};
pub use callback::{QuickJsCallbackValue, QuickJsCopiedValue, QuickJsHostValue, QuickJsValue};
pub use config::QuickJsHostConfig;
pub(crate) use module_loader::{ModuleLoadCallback, ModuleLoader, ModuleNormalizeCallback};
pub(crate) use promise_rejection::PromiseRejectionHandler;
pub use promise_rejection::QuickJsPromiseRejection;
pub(crate) use state::HostState;
pub(crate) use wasi_host::QuickJsWasiHostHandle;
pub use wasi_host::{
    QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat, QuickJsWasiFileType, QuickJsWasiHost,
    QuickJsWasiPrestat, QuickJsWasiWhence,
};

const ERRNO_SUCCESS: i32 = 0;
const ERRNO_BADF: i32 = 8;
const ERRNO_INVAL: i32 = 28;
const ERRNO_NAMETOOLONG: i32 = 37;
const ERRNO_NOENT: i32 = 44;
const ERRNO_NOSYS: i32 = 52;
const ERRNO_NOTCAPABLE: i32 = 76;

#[derive(Clone, Copy)]
enum WasiStdioFd {
    Stdout,
    Stderr,
}

pub(crate) fn define_env_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_get_timezone_offset",
        |caller: Caller<'_, HostState>, _hi: i32, _lo: i32| -> i32 {
            caller.data().config().timezone_offset_seconds()
        },
    )?;
    linker.func_wrap(
        "env",
        "host_interrupt",
        |mut caller: Caller<'_, HostState>| -> i32 {
            if caller.data_mut().interrupt_requested() {
                1
            } else {
                0
            }
        },
    )?;
    promise_rejection::define_import(linker)?;
    module_loader::define_imports(linker)?;
    call::define_import(linker)?;
    Ok(())
}

pub(crate) fn define_wasi_imports(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "clock_time_get",
        |mut caller: Caller<'_, HostState>,
         clock_id: i32,
         _precision: i64,
         result_ptr: i32|
         -> wasmtime::Result<i32> {
            if clock_id != 0 && clock_id != 1 {
                return Ok(ERRNO_NOSYS);
            }
            let memory = caller_memory(&caller)?;
            let time = caller.data().config().clock_time_ns().to_le_bytes();
            memory.write(&mut caller, guest_offset(result_ptr), &time)?;
            Ok(ERRNO_SUCCESS)
        },
    )?;

    fd_write::define_import(linker)?;
    fs::define_imports(linker)?;

    linker.func_wrap(
        "wasi_snapshot_preview1",
        "random_get",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> wasmtime::Result<i32> {
            let memory = caller_memory(&caller)?;
            let byte = caller.data().config().random_byte();
            fill_guest_buffer(
                &memory,
                &mut caller,
                guest_offset(buf_ptr),
                guest_len(buf_len)?,
                byte,
            )?;
            Ok(ERRNO_SUCCESS)
        },
    )?;

    Ok(())
}

fn wasi_stdio_fd(fd: i32) -> Option<WasiStdioFd> {
    match fd {
        1 => Some(WasiStdioFd::Stdout),
        2 => Some(WasiStdioFd::Stderr),
        _ => None,
    }
}

fn caller_memory(caller: &Caller<'_, HostState>) -> wasmtime::Result<Memory> {
    caller
        .data()
        .memory()
        .ok_or_else(|| wasmtime::Error::msg("WASM memory not available to host import yet"))
}
