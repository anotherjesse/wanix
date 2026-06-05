//! WASI Preview 1 imports backed by Wanix semantics.
//!
//! This crate owns the syscall boundary for Wasmtime-hosted Wanix tasks.
//! Filesystem calls resolve through Wanix namespaces and WASI file descriptors
//! instead of delegating namespace behavior to the host operating system or a
//! generic WASI filesystem adapter.

mod config;
mod ctx;
mod error;
mod fd;
mod task_config;

#[cfg(test)]
mod tests;

pub use config::{DEFAULT_CLOCK_TIME_NS, Preopen, WasiConfig, WasiFdObserver, WasiFile};
pub use ctx::{WasiCtx, WasiWhence};
pub use error::Errno;
pub use fd::{
    FileStat, WasiFd, WasiFdStat, WasiFileAccess, WasiFileType, WasiFilestatSetTimes,
    WasiLookupFlags, WasiOpenOptions, WasiPathOpen, WasiPrestat, WasiRights,
};
pub use task_config::task_wasi_config;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix-backed wasi imports";
