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
//! [`WasmTaskDriver`] promotes the runner into a first-class Wanix task driver:
//! a `.wasm` task runs against the task's namespace, cwd, env, argv, and stdio
//! fds, with an observable task exit. The shared linker gives command-style
//! guests (`_start`, fd/path I/O, args/env, clock, exit) blocking reads on
//! device fds and an `fd_read`-only `poll_oneoff` (everything else stays
//! deliberately unsupported) — it is not a general-purpose WASI host.

mod cache;
mod capture;
mod commands;
mod driver;
mod runner;
mod state;
mod task_stdio;

pub use cache::module_cache_dir;
pub use capture::{CaptureFile, host_stderr, host_stdout};
pub use commands::{COMMANDS, SHELL_WASM, command_bin};
pub use driver::WasmTaskDriver;
pub use runner::WasiRunner;
pub use state::WasiState;
