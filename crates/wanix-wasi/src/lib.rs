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

#[cfg(test)]
mod tests;

pub use config::{Preopen, WasiConfig, WasiFdObserver, WasiFile};
pub use ctx::{WasiCtx, WasiWhence};
pub use error::Errno;
pub use fd::{
    FileStat, WasiFd, WasiFdStat, WasiFileAccess, WasiFileType, WasiOpenOptions, WasiPathOpen,
    WasiPrestat, WasiRights,
};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix-backed wasi imports";
