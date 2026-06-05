//! Fd-based trust-boundary verification and atomic writes for cache artifacts.
//!
//! Deserializing a Wasmtime artifact is arbitrary-code-execution-equivalent, so
//! the artifact directory and file are verified through file descriptors
//! (`O_NOFOLLOW` + `fstat`) on Unix before any byte is read. See the crate-level
//! docs for the full trust model.

use std::path::Path;

/// Reads the cached artifact named `artifact` under `cache_dir` only if it can
/// be trusted.
///
/// On Unix the leaf cache directory and the artifact file are both verified via
/// file descriptors (`O_NOFOLLOW` + `fstat`) and the bytes are read from the
/// verified file fd, so no path is re-resolved between check and read. On
/// non-Unix platforms there is no portable fd-ownership model, so the directory
/// check degrades to "is a directory" and the artifact is read by path.
///
/// Returns `None` (treated as a cache miss) on any failed check or I/O error.
#[cfg(unix)]
pub(crate) fn read_trusted_artifact(cache_dir: &Path, artifact: &str) -> Option<Vec<u8>> {
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
    let file_fd = openat(
        dir_fd.as_fd(),
        artifact,
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
pub(crate) fn read_trusted_artifact(cache_dir: &Path, artifact: &str) -> Option<Vec<u8>> {
    if !is_owner_private_dir(cache_dir) {
        return None;
    }
    std::fs::read(cache_dir.join(artifact)).ok()
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
pub(crate) fn ensure_owner_private_dir(dir: &Path) -> bool {
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
pub(crate) fn is_owner_private_dir(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(meta) = std::fs::metadata(dir) else {
        return false;
    };
    if !meta.is_dir() {
        return false;
    }
    let euid = rustix::process::geteuid().as_raw();
    meta.uid() == euid && (meta.mode() & 0o022) == 0
}

#[cfg(not(unix))]
pub(crate) fn is_owner_private_dir(dir: &Path) -> bool {
    dir.is_dir()
}

/// Writes `bytes` to `path` atomically via a unique temp file + rename so a
/// crashed or concurrent writer never leaves a truncated artifact behind.
///
/// The parent directory is (re)created with owner-only permissions and verified
/// owner-private first; a directory that cannot be made private is left
/// untouched and no artifact is written. `key_prefix` disambiguates concurrent
/// writers in the temp name.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], key_prefix: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent()
        && !ensure_owner_private_dir(dir)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "cache directory is not owner-private",
        ));
    }
    // Disambiguate concurrent writers by the artifact key prefix plus the
    // writer's pid; the final rename is atomic so the last writer wins
    // harmlessly.
    let suffix = format!("{key_prefix}.{}.tmp", std::process::id());
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
