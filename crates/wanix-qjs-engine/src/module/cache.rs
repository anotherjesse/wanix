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
//! a private, owner-only directory, and reads the artifact through file
//! descriptors so the checks cannot be raced past:
//!
//! - On creation the directory tree is made with owner-only permissions
//!   (`0o700` on Unix).
//! - On Unix, before any read, the *leaf* cache directory is opened with
//!   `O_NOFOLLOW | O_DIRECTORY` and the resulting fd is `fstat`ed: it must be
//!   owned by the current effective UID and must not be writable by group or
//!   other (`0o022` bits clear). The artifact is then opened relative to that
//!   directory fd with `O_NOFOLLOW` and *its* fd is `fstat`ed: it must be a
//!   regular file, owned by the effective UID, and not group/other-writable.
//!   The bytes are read from that same fd, so no path is re-resolved between the
//!   check and the read (no symlink/TOCTOU swap window).
//! - This verifies the *leaf directory* and the artifact file, but does **not**
//!   walk and verify every ancestor directory. The cache relies on the
//!   owner-private-ancestor assumption documented and asserted in
//!   [`bundled_module_cache_dir`] (the default location lives under an
//!   owner-controlled per-user cache root). A writable ancestor can still swap
//!   the leaf directory, but the fd-based leaf+artifact checks above ensure the
//!   swapped-in directory/artifact must itself be owner-private to be trusted.
//! - A directory or artifact that fails any check is treated as a cache miss
//!   (fresh compile), never trusted.
//!
//! This is why the default cache directory ([`bundled_module_cache_dir`] in the
//! `wanix-qjs` crate) is a per-user cache location rather than a shared,
//! world-writable temp directory. `WANIX_QJS_CACHE_DIR` is an explicit operator
//! opt-in to a trusted path and is still subject to the same fd-based
//! leaf-directory and artifact ownership/permission verification before any
//! artifact is deserialized.
//!
//! On non-Unix platforms there is no portable fd-based ownership model, so the
//! checks degrade to confirming the path is a directory and the caller relies on
//! a per-user default location instead — a weaker guarantee.
//!
//! [`bundled_module_cache_dir`]: ../../../wanix_qjs/fn.bundled_module_cache_dir.html

use std::path::{Path, PathBuf};

use anyhow::Result;
use wasmtime::{Engine, Module};

/// Artifact filename (no directory) for one wasm build.
///
/// The Wasmtime engine embeds its own version/config compatibility marker in
/// the serialized bytes and rejects mismatches on `deserialize`, so keying on
/// the wasm SHA-256 alone is sufficient: an incompatible artifact is detected
/// and recompiled rather than trusted.
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
/// Returns the compiled [`Module`]. Cache read/write failures are non-fatal:
/// the function always falls back to compiling from `bytes`, so a read-only,
/// missing, or untrusted cache directory only forfeits the speedup.
///
/// A *hostile* (untrusted) cache directory is never deserialized: the directory
/// and the artifact file must pass [`read_trusted_artifact`]'s fd-based checks
/// before any artifact read, so a pre-seeded artifact in a world-writable path
/// (or a symlinked/non-regular artifact) is ignored rather than executed.
pub(super) fn load_or_compile(
    engine: &Engine,
    bytes: &[u8],
    wasm_sha256: &[u8; 32],
    cache_dir: &Path,
) -> Result<Module> {
    if let Some(artifact) = read_trusted_artifact(cache_dir, wasm_sha256) {
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
        let _ = write_atomic(&path, &artifact, wasm_sha256);
    }

    Ok(module)
}

/// Reads the cached artifact for `wasm_sha256` only if it can be trusted.
///
/// On Unix the leaf cache directory and the artifact file are both verified via
/// file descriptors (`O_NOFOLLOW` + `fstat`) and the bytes are read from the
/// verified file fd, so no path is re-resolved between check and read. On
/// non-Unix platforms there is no portable fd-ownership model, so the directory
/// check degrades to "is a directory" and the artifact is read by path.
///
/// Returns `None` (treated as a cache miss) on any failed check or I/O error.
#[cfg(unix)]
fn read_trusted_artifact(cache_dir: &Path, wasm_sha256: &[u8; 32]) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::fd::AsFd;

    use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};

    // SAFETY-equivalent: `geteuid` only reads the calling process id and cannot
    // fail.
    let euid = rustix::process::geteuid().as_raw();

    // Open the *leaf* directory without following a final symlink and verify the
    // fd (not the path) is an owner-private directory.
    let dir_fd = open(
        cache_dir,
        OFlags::NOFOLLOW | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let dir_stat = fstat(&dir_fd).ok()?;
    if !stat_is_owner_private(&dir_stat, euid) {
        return None;
    }

    // Open the artifact relative to the verified directory fd, without following
    // a symlink, then verify *that* fd is a regular owner-private file before
    // reading its bytes from the same fd.
    let name = artifact_name(wasm_sha256);
    let file_fd = openat(
        dir_fd.as_fd(),
        name.as_str(),
        OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let file_stat = fstat(&file_fd).ok()?;
    if FileType::from_raw_mode(file_stat.st_mode as _) != FileType::RegularFile
        || !stat_is_owner_private(&file_stat, euid)
    {
        return None;
    }

    let mut file = std::fs::File::from(file_fd);
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(buf)
}

#[cfg(not(unix))]
fn read_trusted_artifact(cache_dir: &Path, wasm_sha256: &[u8; 32]) -> Option<Vec<u8>> {
    if !is_owner_private_dir(cache_dir) {
        return None;
    }
    std::fs::read(artifact_path(cache_dir, wasm_sha256)).ok()
}

/// Returns true when an `fstat`ed entry is owned by `euid` and carries no
/// group/other-write bits (`0o022`).
#[cfg(unix)]
fn stat_is_owner_private(stat: &rustix::fs::Stat, euid: u32) -> bool {
    stat.st_uid == euid && (stat.st_mode as u32 & 0o022) == 0
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
    write_private(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Writes `bytes` to a freshly created file, owner-read/write only (`0o600`) on
/// Unix as defense-in-depth so the temp artifact is never group/other-readable
/// or -writable even briefly before the rename.
#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
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
            read_trusted_artifact(&dir, &key).is_some(),
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
            read_trusted_artifact(&dir, &key).is_none(),
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
            read_trusted_artifact(&dir, &key).is_none(),
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
            read_trusted_artifact(&dir, &key).is_none(),
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
