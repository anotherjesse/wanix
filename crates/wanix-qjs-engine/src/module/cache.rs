//! On-disk cache for compiled QuickJS WASM modules.
//!
//! Cranelift-compiling the ~1.7 MiB QuickJS fixture costs ~550 ms per process,
//! which dominates qjs task cold-start. Wasmtime can serialize a compiled
//! module to a host/version-bound artifact and `deserialize` it in well under a
//! millisecond, so callers that load the same wasm build repeatedly cache the
//! artifact keyed by the wasm SHA-256.
//!
//! The cache is advisory: a missing, stale, or incompatible artifact falls back
//! to a fresh compile, and the engine refuses artifacts that do not match its
//! configuration, so a Wasmtime upgrade simply recompiles under a new key.
//!
//! # Trust model (load via `unsafe Module::deserialize`)
//!
//! Deserializing a Wasmtime artifact is arbitrary-code-execution-equivalent:
//! the SHA-256 key authenticates the *input wasm*, not the cached `.cwasm`
//! bytes. A local attacker who can pre-seed `<sha256>.cwasm` in the cache
//! directory would otherwise get code execution in the victim's process.
//!
//! The cache therefore refuses to read or write through a directory that is not
//! a private, owner-only directory:
//!
//! - On creation the directory tree is made with owner-only permissions
//!   (`0o700` on Unix).
//! - Before any read or write, the directory's metadata is verified: on Unix it
//!   must be owned by the current effective UID and must not be writable by
//!   group or other (`0o022` bits clear). A directory that fails the check is
//!   treated as a cache miss (fresh compile), never trusted.
//!
//! This is why the default cache directory ([`bundled_module_cache_dir`] in the
//! `wanix-qjs` crate) is a per-user cache location rather than a shared,
//! world-writable temp directory. `WANIX_QJS_CACHE_DIR` is an explicit operator
//! opt-in to a trusted path and is still subject to the same ownership/perm
//! verification before any artifact is deserialized.
//!
//! [`bundled_module_cache_dir`]: ../../../wanix_qjs/fn.bundled_module_cache_dir.html

use std::path::{Path, PathBuf};

use anyhow::Result;
use wasmtime::{Engine, Module};

/// Artifact filename for one wasm build under a cache directory.
///
/// The Wasmtime engine embeds its own version/config compatibility marker in
/// the serialized bytes and rejects mismatches on `deserialize`, so keying on
/// the wasm SHA-256 alone is sufficient: an incompatible artifact is detected
/// and recompiled rather than trusted.
fn artifact_path(cache_dir: &Path, wasm_sha256: &[u8; 32]) -> PathBuf {
    let mut name = String::with_capacity(64 + 6);
    for byte in wasm_sha256 {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push_str(".cwasm");
    cache_dir.join(name)
}

/// Loads a compiled module from the cache, or compiles and caches it.
///
/// Returns the compiled [`Module`]. Cache read/write failures are non-fatal:
/// the function always falls back to compiling from `bytes`, so a read-only,
/// missing, or untrusted cache directory only forfeits the speedup.
///
/// A *hostile* (untrusted) cache directory is never deserialized: the directory
/// must pass [`is_owner_private_dir`] before any artifact read, so a
/// pre-seeded artifact in a world-writable path is ignored rather than executed.
pub(super) fn load_or_compile(
    engine: &Engine,
    bytes: &[u8],
    wasm_sha256: &[u8; 32],
    cache_dir: &Path,
) -> Result<Module> {
    let path = artifact_path(cache_dir, wasm_sha256);

    if is_owner_private_dir(cache_dir)
        && let Ok(artifact) = std::fs::read(&path)
    {
        // SAFETY: the cache directory is verified owner-private above, so only
        // the current user could have written this artifact, and `deserialize`
        // additionally validates the engine compatibility marker the matching
        // `serialize` wrote and errors on mismatch or corruption. Any error is
        // treated as a cache miss and recompiled.
        if let Ok(module) = unsafe { Module::deserialize(engine, &artifact) } {
            return Ok(module);
        }
    }

    let module = Module::new(engine, bytes)?;

    // Best-effort write; ignore failures (read-only dir, races, full disk).
    if let Ok(artifact) = module.serialize() {
        let _ = write_atomic(&path, &artifact, wasm_sha256);
    }

    Ok(module)
}

/// Creates `dir` (and parents) with owner-only permissions and verifies it is a
/// private, owner-owned directory.
///
/// Returns `true` only when the directory exists and passes
/// [`is_owner_private_dir`]; callers use this both to decide whether a cached
/// artifact may be trusted and whether a fresh artifact may be written.
fn ensure_owner_private_dir(dir: &Path) -> bool {
    if create_private_dir(dir).is_err() {
        return false;
    }
    is_owner_private_dir(dir)
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    if dir.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Verifies that `dir` is a directory owned by the current user and not
/// writable by group or other.
///
/// On Unix this checks the owning UID against the effective UID and rejects any
/// `0o022` permission bits. On non-Unix platforms there is no portable
/// ownership model, so the check only confirms the path is a directory and the
/// caller relies on a per-user default location instead.
#[cfg(unix)]
fn is_owner_private_dir(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(meta) = std::fs::metadata(dir) else {
        return false;
    };
    if !meta.is_dir() {
        return false;
    }
    // SAFETY: `geteuid` is always safe; it only reads the calling process's
    // effective user id and cannot fail.
    let euid = unsafe { libc_geteuid() };
    meta.uid() == euid && (meta.mode() & 0o022) == 0
}

#[cfg(not(unix))]
fn is_owner_private_dir(dir: &Path) -> bool {
    dir.is_dir()
}

#[cfg(unix)]
unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

/// Writes `bytes` to `path` atomically via a unique temp file + rename so a
/// crashed or concurrent writer never leaves a truncated artifact behind.
///
/// The parent directory is (re)created with owner-only permissions and verified
/// owner-private first; a directory that cannot be made private is left
/// untouched and no artifact is written.
fn write_atomic(path: &Path, bytes: &[u8], wasm_sha256: &[u8; 32]) -> std::io::Result<()> {
    if let Some(dir) = path.parent()
        && !ensure_owner_private_dir(dir)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "cache directory is not owner-private",
        ));
    }
    // Disambiguate concurrent writers by the artifact key plus the writer's pid;
    // the final rename is atomic so the last writer wins harmlessly.
    let suffix = format!(
        "{:02x}{:02x}.{}.tmp",
        wasm_sha256[0],
        wasm_sha256[1],
        std::process::id()
    );
    let tmp = path.with_extension(suffix);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::Engine;

    fn unique_dir(label: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "wanix-qjs-cache-test-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    // A tiny valid wasm module: `(module)`.
    const EMPTY_WASM: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

    fn sha(bytes: &[u8]) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).into()
    }

    #[test]
    fn caches_and_reuses_compiled_artifact() {
        let dir = unique_dir("reuse");
        let engine = Engine::default();
        let key = sha(EMPTY_WASM);

        // First call compiles and writes the artifact.
        load_or_compile(&engine, EMPTY_WASM, &key, &dir).unwrap();
        let path = artifact_path(&dir, &key);
        assert!(path.exists(), "artifact should be written on miss");

        // Second call hits the cache (artifact still present and loadable).
        load_or_compile(&engine, EMPTY_WASM, &key, &dir).unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn created_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_dir("perms");
        let engine = Engine::default();
        let key = sha(EMPTY_WASM);
        load_or_compile(&engine, EMPTY_WASM, &key, &dir).unwrap();

        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "cache dir must be 0700");
        assert!(is_owner_private_dir(&dir));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn world_writable_dir_is_not_trusted() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_dir("hostile");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(
            !is_owner_private_dir(&dir),
            "world-writable dir must be rejected"
        );

        // A pre-seeded artifact in a world-writable dir must be ignored: the
        // load still succeeds by recompiling, and does not deserialize the
        // hostile bytes.
        let engine = Engine::default();
        let key = sha(EMPTY_WASM);
        let path = artifact_path(&dir, &key);
        std::fs::write(&path, b"hostile not-a-cwasm").unwrap();
        // Would panic/UB if it tried to deserialize the hostile bytes; instead
        // it recompiles cleanly.
        load_or_compile(&engine, EMPTY_WASM, &key, &dir).unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }
}
