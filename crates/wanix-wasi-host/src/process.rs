//! WASI Preview 1 process lifecycle imports.

use wasmtime::{Caller, Error, Linker, Result};

use super::WasiHost;

pub(super) fn register<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()> {
    linker.func_wrap(
        super::MODULE,
        "proc_exit",
        |mut caller: Caller<'_, S>, code: i32| {
            caller.data_mut().on_proc_exit(code);
            // Unwind the guest; the consumer recovers the code from its exit hook.
            Err::<(), _>(Error::msg(format!("proc_exit({code})")))
        },
    )?;
    Ok(())
}
