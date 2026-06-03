//! Snapshot-able QuickJS-on-WASI runtime primitives.
//!
//! This crate hosts a QuickJS-NG WebAssembly reactor with Wasmtime, captures
//! the reactor's linear memory plus the small set of VM pointers needed for
//! restore, and resumes that image in a fresh instance of the same wasm module.
//! Snapshots are VM images, so callers must validate them against the exact
//! [`QuickJsModule`] they were created from before restore.
//!
//! ```no_run
//! use rust_wasi_quickjs::{QuickJsHostConfig, QuickJsModule};
//! use wasmtime::Engine;
//!
//! # fn main() -> anyhow::Result<()> {
//! let engine = Engine::default();
//! let module = QuickJsModule::from_file(&engine, "fixtures/quickjs.wasm")?;
//! let create_config = QuickJsHostConfig::new().with_clock_time_ns(1_700_000_000_000_000_000);
//! let mut runtime = module.create_runtime_with_host_config(create_config)?;
//! runtime.eval_discard("globalThis.answer = 41; queueMicrotask(() => globalThis.answer += 1);")?;
//! runtime.execute_pending_jobs_with_limit(8)?;
//!
//! let bytes = runtime.snapshot()?.try_to_bytes()?;
//! let restore_config = QuickJsHostConfig::new().with_clock_time_ns(1_800_000_000_000_000_000);
//! let mut restored = module.restore_runtime_from_bytes_with_host_config(&bytes, restore_config)?;
//!
//! assert_eq!(restored.eval_number("answer")?, 42.0);
//! assert_eq!(restored.eval_number("Date.now()")?, 1_800_000_000_000.0);
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

mod allocation;
mod binary;
mod bytecode;
mod guest;
mod host;
#[cfg(test)]
mod host_tests;
mod intrinsics;
mod memory;
mod module;
#[cfg(test)]
mod module_tests;
mod runtime;
#[cfg(test)]
mod runtime_cleanup_tests;
#[cfg(test)]
mod runtime_host_tests;
mod snapshot;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod tests;

pub use binary::{QuickJsBinaryValue, QuickJsTypedArrayKind};
pub use bytecode::{QuickJsBytecode, QuickJsBytecodeCompileOptions};
pub use host::{
    QuickJsCallbackValue, QuickJsCopiedValue, QuickJsHostConfig, QuickJsHostValue,
    QuickJsPromiseRejection, QuickJsValue, QuickJsWasiErrno, QuickJsWasiFdStat,
    QuickJsWasiFileStat, QuickJsWasiFileType, QuickJsWasiHost, QuickJsWasiPrestat,
    QuickJsWasiWhence,
};
pub use intrinsics::{QuickJsCreateOptions, QuickJsIntrinsics, QuickJsRestoreOptions};
pub use memory::QuickJsMemoryUsage;
pub use module::QuickJsModule;
pub use runtime::QuickJsRuntime;
pub use snapshot::{Snapshot, SnapshotMetadata};
