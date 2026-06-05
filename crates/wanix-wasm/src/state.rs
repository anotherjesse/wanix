//! Store state for a running WASI command task.

use wanix_wasi::WasiCtx;
use wanix_wasi_host::WasiHost;

/// Store state for a running WASI command: a [`WasiCtx`] plus the deterministic
/// clock and recorded exit code the shared linker needs via [`WasiHost`].
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

impl WasiHost for WasiState {
    fn wasi(&mut self) -> &mut WasiCtx {
        &mut self.ctx
    }

    fn clock_time_ns(&self) -> u64 {
        self.clock_ns
    }

    fn on_proc_exit(&mut self, code: i32) {
        self.exit_code = Some(code);
    }
}
