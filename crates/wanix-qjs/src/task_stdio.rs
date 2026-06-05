//! Re-exports the shared task WASI config builder.
//!
//! The namespace/stdio/argv/env wiring and dynamic fd mirroring live in
//! [`wanix_wasi`] so every WASI task runtime (QuickJS and the compiled-wasm
//! driver) follows one fd-mirroring contract. This module keeps the
//! `crate::task_stdio::task_wasi_config` path stable for qjs callers.

pub(crate) use wanix_wasi::task_wasi_config;
