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

    use rustix::fs::{FileType, Mode, OFlags, fstat, openat};

    // SAFETY-equivalent: `geteuid` only reads the calling process id and cannot
    // fail.
    let euid = rustix::process::geteuid().as_raw();

    // Open + fstat-verify the leaf directory through its fd (no symlink-follow on
    // the final component).
    let dir_fd = open_verified_dir(cache_dir, euid)?;

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

/// Opens the *leaf* cache directory with `O_NOFOLLOW | O_DIRECTORY` and returns
/// its fd only if the fd `fstat`s as an owner-private directory.
///
/// Returning the verified fd lets both the read and write paths operate relative
/// to that exact inode (`openat`/`renameat`), so a symlinked or swapped leaf
/// directory is rejected and no path is re-resolved through a possibly-hostile
/// ancestor between the check and the use.
#[cfg(unix)]
fn open_verified_dir(cache_dir: &Path, euid: u32) -> Option<rustix::fd::OwnedFd> {
    use rustix::fs::{Mode, OFlags, fstat, open};

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
    Some(dir_fd)
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
/// Used by the non-Unix write path; the Unix write path instead verifies the
/// directory through an `O_NOFOLLOW` fd (see [`write_atomic`]).
#[cfg(not(unix))]
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
// On Unix the production read/write paths verify the directory through an
// `O_NOFOLLOW` fd; this path-based check remains only for tests asserting the
// owner-private invariant.
#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
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

/// Writes `artifact` under `cache_dir` atomically via a unique temp file +
/// rename so a crashed or concurrent writer never leaves a truncated artifact
/// behind.
///
/// On Unix the directory is opened `O_NOFOLLOW` and `fstat`-verified
/// owner-private, the temp file is created with `O_CREAT | O_EXCL | O_NOFOLLOW`
/// at mode `0o600` relative to that directory fd, and the rename is a
/// `renameat` relative to the same fd — so the write target is the verified
/// inode and is never re-resolved through a (possibly swapped) ancestor or
/// symlinked leaf. `key_prefix` disambiguates concurrent writers; the final
/// rename is atomic so the last writer wins harmlessly. A directory that cannot
/// be verified owner-private is left untouched and no artifact is written.
#[cfg(unix)]
pub(crate) fn write_atomic(
    cache_dir: &Path,
    artifact: &str,
    bytes: &[u8],
    key_prefix: &str,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::fd::AsFd;

    use rustix::fs::{AtFlags, Mode, OFlags, openat, renameat, unlinkat};

    create_private_dir(cache_dir)?;
    let euid = rustix::process::geteuid().as_raw();
    let dir_fd = open_verified_dir(cache_dir, euid).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "cache directory is not owner-private",
        )
    })?;

    let tmp = format!("{key_prefix}.{}.tmp", std::process::id());
    // Only this user can create entries in the verified owner-private directory,
    // so any same-named temp is our own stale leftover; clear it so the
    // O_EXCL create below succeeds.
    let _ = unlinkat(dir_fd.as_fd(), tmp.as_str(), AtFlags::empty());
    let tmp_fd = openat(
        dir_fd.as_fd(),
        tmp.as_str(),
        OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(std::io::Error::from)?;
    {
        let mut file = std::fs::File::from(tmp_fd);
        file.write_all(bytes)?;
    }
    // Atomic rename within the verified directory: both names resolve relative
    // to the same fstat-verified fd, so no path component is re-resolved.
    renameat(dir_fd.as_fd(), tmp.as_str(), dir_fd.as_fd(), artifact).map_err(std::io::Error::from)
}

#[cfg(not(unix))]
pub(crate) fn write_atomic(
    cache_dir: &Path,
    artifact: &str,
    bytes: &[u8],
    key_prefix: &str,
) -> std::io::Result<()> {
    if !ensure_owner_private_dir(cache_dir) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "cache directory is not owner-private",
        ));
    }
    let tmp = cache_dir.join(format!("{key_prefix}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, cache_dir.join(artifact))
}
