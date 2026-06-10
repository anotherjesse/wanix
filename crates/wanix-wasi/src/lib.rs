//! WASI Preview 1 imports backed by Wanix semantics.
//!
//! This crate owns the syscall boundary for Wasmtime-hosted Wanix tasks.
//! Filesystem calls resolve through Wanix namespaces and WASI file descriptors
//! instead of delegating namespace behavior to the host operating system or a
//! generic WASI filesystem adapter.
//!
//! ## Blocking stdio reads
//!
//! [`WasiCtx::fd_read`] on a stdio fd is *blocking* (ADR 0010 tier 2): a
//! queue-backed device attached to fd 0 (`#term` program side, `#pipe`) parks
//! the calling host thread with bounded-backoff readiness polling (see
//! [`wait`]) until data arrives or end-of-stream is decidable, so a resident
//! guest can loop `read(0, …)` on a line stream without busy-spinning or
//! seeing spurious empty reads. Regular byte files always report ready —
//! including at EOF — and dynamic guest-opened fds stay nonblocking, because
//! guest drain loops (read a device queue until 0) depend on empty reads
//! returning immediately.
//!
//! Parks are kill-aware: a [`CancelToken`] (injected by [`task_wasi_config`],
//! probing the task's kill flag) is checked on every park wake, and a
//! cancelled blocking read returns [`Errno::Intr`] instead of waiting for
//! readiness that may never come — see [`cancel`](CancelToken).

mod cancel;
mod config;
mod ctx;
mod error;
mod fd;
mod task_config;
pub mod wait;

#[cfg(test)]
mod tests;

pub use cancel::CancelToken;
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
