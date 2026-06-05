//! Per-user, owner-private cache-directory resolution shared by the qjs and wasm
//! runtimes.
//!
//! Both runtimes load Wasmtime serialized artifacts via `unsafe
//! Module::deserialize` (see [`load_or_compile`](crate::load_or_compile)), so the
//! default cache location must be a *per-user, owner-private* directory rather
//! than a shared, world-writable temp directory: a local attacker must not be
//! able to pre-seed a hostile artifact the victim would deserialize.
//!
//! # Owner-private-ancestor assumption
//!
//! The cache layer verifies the *leaf* cache directory and the artifact file via
//! file descriptors (`O_NOFOLLOW` + `fstat`), but does not walk and re-verify
//! every ancestor directory on each read. The resolver here keeps the chosen
//! location's ancestors owner-private:
//!
//! - The default location lives under the platform per-user cache root
//!   (`$XDG_CACHE_HOME` / `$HOME/.cache` / `%LOCALAPPDATA%`), whose ancestors are
//!   owned and controlled by the current user by construction.
//! - The last-resort fallback is a UID-scoped subdirectory of the system temp
//!   dir; the system temp dir itself is typically sticky (`0o1777`), so a peer
//!   user cannot rename or replace our UID-scoped subdirectory, and the leaf
//!   fd-based check rejects any subdirectory that is not owner-private.
//!
//! The `env_override` variable is an explicit operator opt-in: the operator
//! asserts the supplied path's ancestors are owner-private. Even then, the
//! fd-based leaf and artifact checks reject any directory or artifact that is not
//! owner-private, so a misconfigured override forfeits the speedup rather than
//! deserializing a hostile artifact.

use std::ffi::OsString;
use std::path::PathBuf;

/// Resolves an owner-private cache directory for compiled module artifacts.
///
/// Resolution order:
///
/// 1. The `env_override` environment variable if set and non-empty — an explicit
///    operator opt-in to a trusted path. The cache layer still verifies
///    leaf-directory ownership/permissions (and, on Unix, the artifact file's own
///    ownership/permissions via an `O_NOFOLLOW` fd) before reading any artifact,
///    so an unsafe override only forfeits the speedup.
/// 2. A per-user cache directory derived from the platform's user cache home
///    (`XDG_CACHE_HOME` or `$HOME/.cache` on Unix, `%LOCALAPPDATA%` on Windows),
///    joined with `wanix` and `subdir`.
/// 3. A UID-scoped subdirectory of the system temp dir as a last resort, so the
///    default is never a shared, world-writable, predictable path.
///
/// `subdir` distinguishes runtimes that share this resolver (e.g.
/// `qjs-module-cache` vs `wasm-module-cache`).
#[must_use]
pub fn owner_private_cache_dir(env_override: &str, subdir: &str) -> PathBuf {
    let override_dir = std::env::var_os(env_override).filter(|value| !value.is_empty());
    cache_dir_from_parts(override_dir, user_cache_home(), subdir)
}

fn cache_dir_from_parts(
    override_dir: Option<OsString>,
    user_cache_home: Option<PathBuf>,
    subdir: &str,
) -> PathBuf {
    if let Some(dir) = override_dir {
        return PathBuf::from(dir);
    }
    user_cache_home
        .unwrap_or_else(uid_scoped_temp_dir)
        .join("wanix")
        .join(subdir)
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
fn non_empty_var(key: &str) -> Option<OsString> {
    std::env::var_os(key).filter(|value| !value.is_empty())
}

/// A UID-scoped temp subdirectory: still per-user (the cache layer enforces
/// owner-private perms), never the bare shared temp path.
fn uid_scoped_temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("wanix-module-cache-{}", current_user_id()))
}

#[cfg(unix)]
fn current_user_id() -> u64 {
    u64::from(rustix::process::geteuid().as_raw())
}

#[cfg(not(unix))]
fn current_user_id() -> u64 {
    u64::from(std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins_over_default() {
        assert_eq!(
            cache_dir_from_parts(
                Some("/tmp/explicit-cache".into()),
                Some(PathBuf::from("/tmp/cache-home")),
                "qjs-module-cache",
            ),
            PathBuf::from("/tmp/explicit-cache")
        );
    }

    #[test]
    fn default_is_namespaced_under_cache_home() {
        let dir = cache_dir_from_parts(
            None,
            Some(PathBuf::from("/tmp/cache-home")),
            "wasm-module-cache",
        );
        assert_eq!(
            dir,
            PathBuf::from("/tmp/cache-home")
                .join("wanix")
                .join("wasm-module-cache")
        );
        assert_ne!(dir, std::env::temp_dir());
        assert!(dir.to_string_lossy().contains("wanix"));
    }

    #[test]
    fn fallback_is_uid_scoped_temp() {
        let fallback = cache_dir_from_parts(None, None, "qjs-module-cache");
        assert_eq!(
            fallback,
            uid_scoped_temp_dir().join("wanix").join("qjs-module-cache")
        );
    }

    #[test]
    fn subdir_separates_runtimes() {
        let qjs = cache_dir_from_parts(None, Some(PathBuf::from("/c")), "qjs-module-cache");
        let wasm = cache_dir_from_parts(None, Some(PathBuf::from("/c")), "wasm-module-cache");
        assert_ne!(qjs, wasm);
    }
}
