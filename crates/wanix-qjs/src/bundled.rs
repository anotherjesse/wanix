//! Policy for the bundled QuickJS module's on-disk compiled-artifact cache.
//!
//! The compiled-artifact cache loads Wasmtime serialized modules via
//! `unsafe Module::deserialize`, so the directory holding those artifacts is a
//! trust boundary (see `wanix-qjs-engine`'s `module::cache`). This module
//! chooses a *per-user, owner-private* default location rather than a shared,
//! world-writable temp directory, so a local attacker cannot pre-seed a hostile
//! artifact that the victim would deserialize.

use std::path::PathBuf;

/// Directory holding cached compiled artifacts for the bundled QuickJS module.
///
/// Resolution order:
///
/// 1. `WANIX_QJS_CACHE_DIR` if set — an explicit operator opt-in to a trusted
///    path. The cache layer still verifies ownership/permissions before reading
///    any artifact, so an unsafe override only forfeits the speedup.
/// 2. A per-user cache directory derived from the platform's user cache home
///    (`XDG_CACHE_HOME` or `$HOME/.cache` on Unix, `%LOCALAPPDATA%` on Windows).
/// 3. A UID-scoped subdirectory of the system temp dir as a last resort, so the
///    default is never a shared, world-writable, predictable path.
///
/// The cache is reproducible from the bundled fixture, so a cleared cache only
/// costs a recompile.
#[must_use]
pub fn bundled_module_cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WANIX_QJS_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    bundled_module_cache_dir_from_parts(None, user_cache_home())
}

fn bundled_module_cache_dir_from_parts(
    override_dir: Option<std::ffi::OsString>,
    user_cache_home: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return PathBuf::from(dir);
    }
    user_cache_home
        .unwrap_or_else(default_cache_home)
        .join("wanix")
        .join("qjs-module-cache")
}

fn default_cache_home() -> PathBuf {
    uid_scoped_temp_dir()
}

/// Per-user cache home, if the platform exposes one.
#[cfg(unix)]
fn user_cache_home() -> Option<PathBuf> {
    if let Some(xdg) = non_empty_var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg));
    }
    non_empty_var("HOME").map(|home| PathBuf::from(home).join(".cache"))
}

#[cfg(windows)]
fn user_cache_home() -> Option<PathBuf> {
    non_empty_var("LOCALAPPDATA").map(PathBuf::from)
}

#[cfg(not(any(unix, windows)))]
fn user_cache_home() -> Option<PathBuf> {
    None
}

#[cfg(any(unix, windows))]
fn non_empty_var(key: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(key).filter(|value| !value.is_empty())
}

/// A UID-scoped temp subdirectory: still per-user (the cache layer enforces
/// owner-private perms), never the bare shared temp path.
fn uid_scoped_temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("wanix-qjs-cache-{}", current_user_id()))
}

#[cfg(unix)]
fn current_user_id() -> u64 {
    // SAFETY: `geteuid` only reads the calling process's effective user id and
    // cannot fail.
    u64::from(unsafe { geteuid() })
}

#[cfg(unix)]
unsafe extern "C" {
    fn geteuid() -> u32;
}

#[cfg(not(unix))]
fn current_user_id() -> u64 {
    u64::from(std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_override_then_per_user_default() {
        assert_eq!(
            bundled_module_cache_dir_from_parts(
                Some("/tmp/explicit-qjs-cache".into()),
                Some(PathBuf::from("/tmp/cache-home")),
            ),
            PathBuf::from("/tmp/explicit-qjs-cache")
        );

        let dir = bundled_module_cache_dir_from_parts(None, Some(PathBuf::from("/tmp/cache-home")));
        assert_ne!(dir, std::env::temp_dir());
        assert_eq!(
            dir,
            PathBuf::from("/tmp/cache-home")
                .join("wanix")
                .join("qjs-module-cache")
        );
        assert!(
            dir.to_string_lossy().contains("wanix"),
            "default cache dir should be namespaced: {dir:?}"
        );

        let fallback = bundled_module_cache_dir_from_parts(None, None);
        assert_eq!(
            fallback,
            uid_scoped_temp_dir().join("wanix").join("qjs-module-cache")
        );
    }
}
