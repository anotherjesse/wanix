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
//!
//! This is a Tier-1 demo/bench runner: a standalone WASI command host, not yet a
//! Wanix task driver. The shared linker registers `poll_oneoff` as `ERRNO_NOSYS`,
//! so the runner supports command-style guests (`_start`, fd/path I/O, args/env,
//! clock, exit) with no poll readiness — it is not a general-purpose WASI host.

mod capture;
mod runner;
mod state;

pub use capture::{CaptureFile, host_stderr, host_stdout};
pub use runner::WasiRunner;
pub use state::WasiState;
