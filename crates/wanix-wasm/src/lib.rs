//! Generic WASI Preview 1 task runner for Rust Wanix.
//!
//! This crate runs an arbitrary `wasm32-wasi` command module (compiled from
//! Rust, Go, C, Zig, …) on Wasmtime, with its WASI syscalls backed by a Wanix
//! [`Namespace`](wanix_vfs::Namespace) through the engine-agnostic
//! [`WasiCtx`](wanix_wasi::WasiCtx). It is the "compiled-to-wasm task" sibling of
//! the QuickJS `qjs` task: same sandbox, same namespace/VFS, near-native speed.
//!
//! Two tasks (a `qjs` task and a `wanix-wasm` task) that are built from the same
//! `Namespace` share one filesystem — writes by one are visible to the other.

use std::sync::{Arc, Mutex};

use wanix_fs::{File, FileType, FsResult, Metadata};
use wanix_wasi::{WasiConfig, WasiCtx};
use wasmtime::error::Context as _;
use wasmtime::{Engine, Error, Linker, Module, Result, Store};

mod imports;

pub use imports::WasiState;

/// A compiled `wasm32-wasi` command module ready to run as Wanix tasks.
pub struct WasiRunner {
    engine: Engine,
    module: Module,
}

impl WasiRunner {
    /// Compiles a `wasm32-wasi` module from bytes with a default engine.
    ///
    /// # Errors
    ///
    /// Returns an error if Wasmtime cannot compile the bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).context("failed to compile wasm module")?;
        Ok(Self { engine, module })
    }

    /// Runs the module's `_start` entry with WASI backed by `config`'s namespace.
    ///
    /// Returns the process exit code: `0` for a normal return, or the code passed
    /// to `proc_exit`.
    ///
    /// # Errors
    ///
    /// Returns an error if the WASI config is invalid, the module is missing a
    /// `_start` export, or a trap other than `proc_exit` occurs.
    pub fn run(&self, config: WasiConfig) -> Result<i32> {
        let clock_ns = config.clock_time_ns();
        let ctx = WasiCtx::try_new(config)
            .map_err(|err| Error::msg(format!("invalid WASI config: {err:?}")))?;
        let mut store = Store::new(&self.engine, WasiState::new(ctx, clock_ns));

        let mut linker = Linker::new(&self.engine);
        imports::add_to_linker(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &self.module)
            .context("failed to instantiate wasm module")?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .context("module has no _start export (not a WASI command)")?;

        match start.call(&mut store, ()) {
            Ok(()) => Ok(store.data().exit_code().unwrap_or(0)),
            Err(err) => match store.data().exit_code() {
                // A clean proc_exit unwinds the guest via a trap; recover the code.
                Some(code) => Ok(code),
                None => Err(err).context("wasm task trapped"),
            },
        }
    }
}

/// An in-memory [`File`] that captures everything written to it.
///
/// Useful as a stdout/stderr sink so a host can read back what a guest printed.
#[derive(Clone, Default)]
pub struct CaptureFile {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl CaptureFile {
    /// Creates an empty capture file.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the bytes written so far as a lossy UTF-8 string.
    #[must_use]
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.buffer.lock().expect("capture lock")).into_owned()
    }
}

impl File for CaptureFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.buffer
            .lock()
            .expect("capture lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn write_ready(&self) -> FsResult<bool> {
        Ok(true)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o644))
    }
}
