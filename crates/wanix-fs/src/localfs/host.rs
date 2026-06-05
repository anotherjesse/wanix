#[cfg(unix)]
use std::ffi::OsStr;
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use crate::{FileType, FsError, FsResult, Metadata, MetadataTimes};

use super::map_io_error;

#[cfg(not(unix))]
const READ_ONLY_FILE_MODE: u32 = 0o444;
#[cfg(not(unix))]
const DEFAULT_DIR_MODE: u32 = 0o755;
#[cfg(not(unix))]
const DEFAULT_FILE_MODE: u32 = 0o644;
#[cfg(unix)]
const PERMISSION_MODE_MASK: u32 = 0o7777;
#[cfg(not(unix))]
const WRITE_PERMISSION_BITS: u32 = 0o222;

pub(super) fn metadata_from_host(metadata: &fs::Metadata) -> Metadata {
    let file_type = if metadata.is_dir() {
        FileType::Directory
    } else if metadata.is_file() {
        FileType::File
    } else if metadata.file_type().is_symlink() {
        FileType::Symlink
    } else {
        FileType::File
    };
    let times = MetadataTimes::new(
        metadata_accessed_time_ns(metadata),
        metadata_modified_time_ns(metadata),
        metadata_changed_time_ns(metadata),
    );
    Metadata::new_with_links(
        file_type,
        metadata.len(),
        metadata_mode(metadata),
        metadata_link_count(metadata),
        times,
    )
}

#[cfg(unix)]
fn metadata_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn metadata_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        READ_ONLY_FILE_MODE
    } else if metadata.is_dir() {
        DEFAULT_DIR_MODE
    } else {
        DEFAULT_FILE_MODE
    }
}

#[cfg(unix)]
fn metadata_link_count(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    metadata.nlink()
}

#[cfg(not(unix))]
fn metadata_link_count(_metadata: &fs::Metadata) -> u64 {
    1
}

#[cfg(unix)]
pub(super) fn set_host_permissions(path: &Path, permissions: u32) -> FsResult<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(
        path,
        fs::Permissions::from_mode(permissions & PERMISSION_MODE_MASK),
    )
    .map_err(map_io_error)
}

#[cfg(not(unix))]
pub(super) fn set_host_permissions(path: &Path, permissions: u32) -> FsResult<()> {
    let mut current = fs::metadata(path).map_err(map_io_error)?.permissions();
    current.set_readonly(permissions & WRITE_PERMISSION_BITS == 0);
    fs::set_permissions(path, current).map_err(map_io_error)
}

#[cfg(unix)]
fn metadata_accessed_time_ns(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    unix_time_ns(metadata.atime(), metadata.atime_nsec())
}

#[cfg(unix)]
fn metadata_modified_time_ns(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    unix_time_ns(metadata.mtime(), metadata.mtime_nsec())
}

#[cfg(unix)]
fn metadata_changed_time_ns(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;

    unix_time_ns(metadata.ctime(), metadata.ctime_nsec())
}

#[cfg(unix)]
fn unix_time_ns(secs: i64, nanos: i64) -> u64 {
    let Ok(secs) = u64::try_from(secs) else {
        return 0;
    };
    let Ok(nanos) = u64::try_from(nanos) else {
        return 0;
    };
    secs.checked_mul(1_000_000_000)
        .and_then(|base| base.checked_add(nanos))
        .unwrap_or(0)
}

#[cfg(not(unix))]
fn metadata_accessed_time_ns(metadata: &fs::Metadata) -> u64 {
    system_time_ns(metadata.accessed().ok())
}

#[cfg(not(unix))]
fn metadata_modified_time_ns(metadata: &fs::Metadata) -> u64 {
    system_time_ns(metadata.modified().ok())
}

#[cfg(not(unix))]
fn metadata_changed_time_ns(metadata: &fs::Metadata) -> u64 {
    system_time_ns(metadata.created().ok())
}

#[cfg(not(unix))]
fn system_time_ns(time: Option<std::time::SystemTime>) -> u64 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
        .unwrap_or(0)
}

pub(super) fn system_time_from_ns(timestamp_ns: u64) -> FsResult<std::time::SystemTime> {
    UNIX_EPOCH
        .checked_add(Duration::from_nanos(timestamp_ns))
        .ok_or(FsError::InvalidTime)
}

#[cfg(unix)]
pub(super) fn pathbuf_into_bytes(path: PathBuf) -> FsResult<Vec<u8>> {
    Ok(path.into_os_string().into_vec())
}

#[cfg(not(unix))]
pub(super) fn pathbuf_into_bytes(path: PathBuf) -> FsResult<Vec<u8>> {
    path.into_os_string()
        .into_string()
        .map(|path| path.into_bytes())
        .map_err(|_| FsError::InvalidPath("<non-utf8 host symlink target>".to_owned()))
}

#[cfg(unix)]
pub(super) fn create_symlink(target: &[u8], host_path: &Path) -> FsResult<()> {
    std::os::unix::fs::symlink(OsStr::from_bytes(target), host_path).map_err(map_io_error)
}

#[cfg(not(unix))]
pub(super) fn create_symlink(_target: &[u8], _host_path: &Path) -> FsResult<()> {
    Err(FsError::NotSupported)
}
