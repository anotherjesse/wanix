//! Policy for the bundled QuickJS module's on-disk compiled-artifact cache.
//!
//! The compiled-artifact cache loads Wasmtime serialized modules via
//! `unsafe Module::deserialize`, so the directory holding those artifacts is a
//! trust boundary (see the [`wanix_module_cache`] crate for the fd-based
//! verification model shared by the qjs and wasm runtimes). This module only
//! pins the bundled-qjs policy: the `WANIX_QJS_CACHE_DIR` override variable and
//! the `qjs-module-cache` subdirectory under the per-user cache root.

use std::path::PathBuf;

/// Environment variable that overrides the bundled QuickJS module cache
/// directory. An explicit operator opt-in to a trusted path; the cache layer
/// still verifies leaf-directory and artifact ownership/permissions before
/// reading any artifact.
const CACHE_DIR_ENV: &str = "WANIX_QJS_CACHE_DIR";

/// Subdirectory (under the per-user cache root) for bundled-qjs artifacts,
/// distinct from the wasm runner's cache so the two runtimes never collide.
const CACHE_SUBDIR: &str = "qjs-module-cache";

/// Directory holding cached compiled artifacts for the bundled QuickJS module.
///
/// Resolution (see [`wanix_module_cache::owner_private_cache_dir`]):
///
/// 1. `WANIX_QJS_CACHE_DIR` if set — an explicit operator opt-in to a trusted
///    path. The cache layer still verifies leaf-directory ownership/permissions
///    (and, on Unix, the artifact file's own ownership/permissions via an
///    `O_NOFOLLOW` fd) before reading any artifact, so an unsafe override only
///    forfeits the speedup.
/// 2. A per-user cache directory derived from the platform's user cache home
///    (`XDG_CACHE_HOME` or `$HOME/.cache` on Unix, `%LOCALAPPDATA%` on Windows).
/// 3. A UID-scoped subdirectory of the system temp dir as a last resort, so the
///    default is never a shared, world-writable, predictable path.
///
/// The cache is reproducible from the bundled fixture, so a cleared cache only
/// costs a recompile.
#[must_use]
pub fn bundled_module_cache_dir() -> PathBuf {
    wanix_module_cache::owner_private_cache_dir(CACHE_DIR_ENV, CACHE_SUBDIR)
}
