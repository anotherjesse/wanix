//! Policy for the bundled QuickJS module's on-disk compiled-artifact cache.

use std::path::PathBuf;

/// Directory holding cached compiled artifacts for the bundled QuickJS module.
///
/// Honors `WANIX_QJS_CACHE_DIR`, else a stable temp subdirectory; the cache is
/// reproducible from the fixture, so a cleared temp only costs a recompile.
#[must_use]
pub fn bundled_module_cache_dir() -> PathBuf {
    std::env::var_os("WANIX_QJS_CACHE_DIR").map_or_else(
        || std::env::temp_dir().join("wanix-qjs-module-cache"),
        PathBuf::from,
    )
}
