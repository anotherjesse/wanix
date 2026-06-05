//! Default cache-directory policy for the compiled `.wasm` runner.
//!
//! [`WasiRunner::from_bytes_cached`](crate::WasiRunner::from_bytes_cached) loads
//! a Wasmtime serialized module via `unsafe Module::deserialize`, so the cache
//! directory is a trust boundary (see the [`wanix_module_cache`] crate for the
//! shared fd-based verification model). This module only pins the wasm-runner
//! policy: the `WANIX_WASM_CACHE_DIR` override variable and the
//! `wasm-module-cache` subdirectory under the per-user cache root — distinct from
//! the bundled-qjs `qjs-module-cache` so the two runtimes never collide.

use std::path::PathBuf;

/// Environment variable that overrides the wasm runner's module cache directory.
/// An explicit operator opt-in to a trusted path; the cache layer still verifies
/// leaf-directory and artifact ownership/permissions before reading any artifact.
const CACHE_DIR_ENV: &str = "WANIX_WASM_CACHE_DIR";

/// Subdirectory (under the per-user cache root) for wasm-runner artifacts.
const CACHE_SUBDIR: &str = "wasm-module-cache";

/// Default directory holding cached compiled artifacts for the wasm runner.
///
/// Resolution (see [`wanix_module_cache::owner_private_cache_dir`]):
///
/// 1. `WANIX_WASM_CACHE_DIR` if set — an explicit operator opt-in to a trusted
///    path. The cache layer still verifies leaf-directory and artifact
///    ownership/permissions (on Unix, via `O_NOFOLLOW` fds) before reading any
///    artifact, so an unsafe override only forfeits the speedup.
/// 2. A per-user cache directory derived from the platform's user cache home
///    (`XDG_CACHE_HOME` or `$HOME/.cache` on Unix, `%LOCALAPPDATA%` on Windows).
/// 3. A UID-scoped subdirectory of the system temp dir as a last resort, so the
///    default is never a shared, world-writable, predictable path.
///
/// A cleared cache only costs a recompile of the wasm module.
#[must_use]
pub fn module_cache_dir() -> PathBuf {
    wanix_module_cache::owner_private_cache_dir(CACHE_DIR_ENV, CACHE_SUBDIR)
}
