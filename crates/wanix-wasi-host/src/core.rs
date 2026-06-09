//! WASI Preview 1 imports that do not touch filesystem state.

use wasmtime::{Caller, Linker, Result};

use super::ERRNO_SUCCESS;
use super::WasiHost;
use super::mem::{memory, write_bytes, write_u64};

pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    let m = super::MODULE;

    linker.func_wrap(m, "sched_yield", |_: Caller<'_, S>| ERRNO_SUCCESS)?;
    linker.func_wrap(
        m,
        "random_get",
        |mut caller: Caller<'_, S>, buf: i32, len: i32| -> Result<i32> {
            let mem = memory(&mut caller)?;
            // Deterministic fill; this runner targets reproducible tasks.
            let deterministic_bytes = vec![7u8; len.max(0) as usize];
            write_bytes(&mem, &mut caller, buf, &deterministic_bytes)?;
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
    Ok(())
}
