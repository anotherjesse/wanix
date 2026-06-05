//! Shared owner-private on-disk cache for Wasmtime-compiled modules.
//!
//! Cranelift-compiling a large WASM module (e.g. the ~1.7 MiB QuickJS fixture)
//! costs hundreds of milliseconds per process, which dominates task cold-start.
//! Wasmtime can serialize a compiled module to a host/version-bound artifact and
//! `deserialize` it in well under a millisecond, so callers that load the same
//! wasm build repeatedly cache the artifact keyed by the wasm SHA-256.
//!
//! Both Wanix WASI runtimes use this single implementation:
//!
//! - `wanix-qjs-engine`'s `QuickJsModule::from_bytes_cached` caches the bundled
//!   QuickJS module.
//! - `wanix-wasm`'s `WasiRunner::from_bytes_cached` caches arbitrary
//!   `wasm32-wasi` command modules.
//!
//! The cache is advisory: a missing, stale, or incompatible artifact falls back
//! to a fresh compile, and the engine refuses artifacts that do not match its
//! configuration, so a Wasmtime upgrade simply recompiles under a new key.
//!
//! # Trust model (load via `unsafe Module::deserialize`)
//!
//! Deserializing a Wasmtime artifact is arbitrary-code-execution-equivalent: the
//! SHA-256 key authenticates the *input wasm*, not the cached `.cwasm` bytes. A
//! local attacker who can pre-seed `<sha256>.cwasm` in the cache directory would
//! otherwise get code execution in the victim's process.
//!
//! The cache therefore refuses to read or write through a directory that is not a
//! private, owner-only directory, and reads the artifact through file descriptors
//! so the checks cannot be raced past:
//!
//! - On creation the directory tree is made with owner-only permissions
//!   (`0o700` on Unix).
//! - On Unix, before any read, the *leaf* cache directory is opened with
//!   `O_NOFOLLOW | O_DIRECTORY` and the resulting fd is `fstat`ed: it must be
//!   owned by the current effective UID and must not be writable by group or
//!   other (`0o022` bits clear). The artifact is then opened relative to that
//!   directory fd with `O_NOFOLLOW` and *its* fd is `fstat`ed: it must be a
//!   regular file, owned by the effective UID, and not group/other-writable. The
//!   bytes are read from that same fd, so no path is re-resolved between the check
//!   and the read (no symlink/TOCTOU swap window).
//! - This verifies the *leaf directory* and the artifact file, but does **not**
//!   walk and verify every ancestor directory. The cache relies on the
//!   owner-private-ancestor assumption documented in [`owner_private_cache_dir`]
//!   (the default location lives under an owner-controlled per-user cache root).
//!   A writable ancestor can still swap the leaf directory, but the fd-based
//!   leaf+artifact checks ensure the swapped-in directory/artifact must itself be
//!   owner-private to be trusted.
//! - A directory or artifact that fails any check is treated as a cache miss
//!   (fresh compile), never trusted.
//!
//! On non-Unix platforms there is no portable fd-based ownership model, so the
//! checks degrade to confirming the path is a directory and the caller relies on
//! a per-user default location instead — a weaker guarantee.

mod dir;
mod trust;

use std::path::{Path, PathBuf};

use anyhow::Result;
use wasmtime::{Engine, Module};

pub use dir::owner_private_cache_dir;

/// Artifact filename (no directory) for one wasm build.
///
/// The Wasmtime engine embeds its own version/config compatibility marker in the
/// serialized bytes and rejects mismatches on `deserialize`, so keying on the
/// wasm SHA-256 alone is sufficient: an incompatible artifact is detected and
/// recompiled rather than trusted.
fn artifact_name(wasm_sha256: &[u8; 32]) -> String {
    let mut name = String::with_capacity(64 + 6);
    for byte in wasm_sha256 {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push_str(".cwasm");
    name
}

/// Full on-disk path of an artifact under a cache directory.
fn artifact_path(cache_dir: &Path, wasm_sha256: &[u8; 32]) -> PathBuf {
    cache_dir.join(artifact_name(wasm_sha256))
}

/// Loads a compiled module from the cache, or compiles and caches it.
///
/// Returns the compiled [`Module`]. Cache read/write failures are non-fatal: the
/// function always falls back to compiling from `bytes`, so a read-only, missing,
/// or untrusted cache directory only forfeits the speedup.
///
/// A *hostile* (untrusted) cache directory is never deserialized: the directory
/// and the artifact file must pass the fd-based checks (see crate docs) before
/// any artifact read, so a pre-seeded artifact in a world-writable path (or a
/// symlinked/non-regular artifact) is ignored rather than executed.
///
/// # Errors
///
/// Returns an error only if a fresh compile (`Module::new`) fails.
pub fn load_or_compile(
    engine: &Engine,
    bytes: &[u8],
    wasm_sha256: &[u8; 32],
    cache_dir: &Path,
) -> Result<Module> {
    let name = artifact_name(wasm_sha256);
    if let Some(artifact) = trust::read_trusted_artifact(cache_dir, &name) {
        // SAFETY: the artifact bytes were read from a regular file opened
        // (`O_NOFOLLOW`) relative to an `fstat`-verified owner-private directory
        // fd, and the file's own fd was `fstat`-verified to be a regular file
        // owned by the effective UID with no group/other-write bits, so only the
        // current user could have written it. `deserialize` additionally
        // validates the engine compatibility marker the matching `serialize`
        // wrote and errors on mismatch or corruption. Any error is treated as a
        // cache miss and recompiled.
        if let Ok(module) = unsafe { Module::deserialize(engine, &artifact) } {
            return Ok(module);
        }
    }

    let module = Module::new(engine, bytes)?;

    // Best-effort write; ignore failures (read-only dir, races, full disk).
    if let Ok(artifact) = module.serialize() {
        let path = artifact_path(cache_dir, wasm_sha256);
        let key_prefix = format!("{:02x}{:02x}", wasm_sha256[0], wasm_sha256[1]);
        let _ = trust::write_atomic(&path, &artifact, &key_prefix);
    }

    Ok(module)
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
            "wanix-module-cache-test-{label}-{}-{nonce}",
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
        assert!(trust::is_owner_private_dir(&dir));

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
            !trust::is_owner_private_dir(&dir),
            "world-writable dir must be rejected"
        );

        // A pre-seeded artifact in a world-writable dir must be ignored: the load
        // still succeeds by recompiling, and does not deserialize the hostile
        // bytes.
        let engine = Engine::default();
        let key = sha(EMPTY_WASM);
        let path = artifact_path(&dir, &key);
        std::fs::write(&path, b"hostile not-a-cwasm").unwrap();
        // Would panic/UB if it tried to deserialize the hostile bytes; instead it
        // recompiles cleanly.
        load_or_compile(&engine, EMPTY_WASM, &key, &dir).unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Writes a real, deserializable artifact into a fresh owner-private cache
    /// dir so that any later rejection is attributable to the trust-boundary
    /// check rather than to deserialize failing on bogus bytes.
    #[cfg(unix)]
    fn seed_valid_artifact(label: &str) -> (PathBuf, [u8; 32]) {
        let dir = unique_dir(label);
        let engine = Engine::default();
        let key = sha(EMPTY_WASM);
        load_or_compile(&engine, EMPTY_WASM, &key, &dir).unwrap();
        assert!(
            trust::read_trusted_artifact(&dir, &artifact_name(&key)).is_some(),
            "freshly written artifact must be trusted"
        );
        (dir, key)
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_artifact_is_rejected() {
        let (dir, key) = seed_valid_artifact("symlink");
        let path = artifact_path(&dir, &key);

        // Replace the real artifact with a symlink that points at a valid
        // artifact stored elsewhere. `O_NOFOLLOW` on the final component must
        // refuse to open it, so the entry is treated as a cache miss.
        let real = dir.join("real.cwasm");
        std::fs::rename(&path, &real).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();

        assert!(
            trust::read_trusted_artifact(&dir, &artifact_name(&key)).is_none(),
            "symlinked artifact must not be trusted"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_artifact_is_rejected() {
        let (dir, key) = seed_valid_artifact("non-regular");
        let path = artifact_path(&dir, &key);

        // Replace the artifact with a directory of the same name: even though it
        // opens (no symlink) and is owner-private, it is not a regular file.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert!(
            trust::read_trusted_artifact(&dir, &artifact_name(&key)).is_none(),
            "non-regular artifact must not be trusted"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn group_or_other_writable_artifact_is_rejected() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, key) = seed_valid_artifact("writable-artifact");
        let path = artifact_path(&dir, &key);

        // Group/other-writable artifact: a peer could have tampered with it, so
        // the fd-based `fstat` check rejects it even though it is a regular file
        // owned by us in an owner-private directory.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();

        assert!(
            trust::read_trusted_artifact(&dir, &artifact_name(&key)).is_none(),
            "group/other-writable artifact must not be trusted"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn temp_artifact_is_written_owner_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_dir("temp-mode");
        let engine = Engine::default();
        let key = sha(EMPTY_WASM);
        load_or_compile(&engine, EMPTY_WASM, &key, &dir).unwrap();

        // The final artifact inherits the temp file's 0o600 mode after rename.
        let mode = std::fs::metadata(artifact_path(&dir, &key))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o077,
            0,
            "artifact must not be group/other-accessible"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
