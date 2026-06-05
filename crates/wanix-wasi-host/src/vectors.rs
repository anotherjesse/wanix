//! WASI Preview 1 argv and environment vector imports.

use wasmtime::{Caller, Linker, Result};

use super::WasiHost;
use super::mem::{write_vec_buffer, write_vec_sizes};

pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::MODULE;

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
    Ok(())
}
