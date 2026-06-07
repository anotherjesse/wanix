//! Error and errno mapping for server-facing 9P replies.

use std::error::Error;
use std::fmt;

use wanix_fs::FsError;
use wanix_protocol::P9Error;

pub(crate) const EBADF: u32 = 9;
pub(crate) const EACCES: u32 = 13;
pub(crate) const EEXIST: u32 = 17;
pub(crate) const ENOTDIR: u32 = 20;
pub(crate) const EISDIR: u32 = 21;
pub(crate) const EINVAL: u32 = 22;
pub(crate) const ENOSYS: u32 = 38;
pub(crate) const ENOTEMPTY: u32 = 39;
/// The named extended attribute does not exist (Linux `ENODATA`).
///
/// Returned by `Txattrwalk` for a `cas.hash` probe on a file that has no
/// content-addressed hash to offer (small file, mid-write, or a filesystem with
/// no blob backing), so a CAS-aware client cleanly falls back to `Tread`.
pub(crate) const ENODATA: u32 = 61;
pub(crate) const EOPNOTSUPP: u32 = 95;

/// Error returned when a request is too malformed to turn into a 9P reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanix9pError {
    /// The request payload failed protocol decoding.
    Protocol(P9Error),
    /// A filesystem path cannot be represented as a normalized Wanix path.
    InvalidPath(String),
}

impl fmt::Display for Wanix9pError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(f, "9P protocol error: {error}"),
            Self::InvalidPath(path) => write!(f, "invalid 9P walk path: {path}"),
        }
    }
}

impl Error for Wanix9pError {}

impl From<P9Error> for Wanix9pError {
    fn from(error: P9Error) -> Self {
        Self::Protocol(error)
    }
}

pub(crate) fn errno_for_fs(error: &FsError) -> u32 {
    match error {
        FsError::InvalidPath(_) | FsError::InvalidOffset | FsError::InvalidTime => EINVAL,
        FsError::NotFound => 2,
        FsError::NotSupported => EOPNOTSUPP,
        FsError::PermissionDenied => EACCES,
        FsError::AlreadyExists => EEXIST,
        FsError::NotDirectory => ENOTDIR,
        FsError::IsDirectory => EISDIR,
        FsError::InvalidFd => EBADF,
        FsError::NotEmpty => ENOTEMPTY,
        FsError::Other(_) => EINVAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_errors_map_to_stable_9p_errno_values() {
        let cases = [
            (FsError::InvalidPath("..".to_owned()), EINVAL),
            (FsError::InvalidOffset, EINVAL),
            (FsError::InvalidTime, EINVAL),
            (FsError::NotFound, 2),
            (FsError::NotSupported, EOPNOTSUPP),
            (FsError::PermissionDenied, EACCES),
            (FsError::AlreadyExists, EEXIST),
            (FsError::NotDirectory, ENOTDIR),
            (FsError::IsDirectory, EISDIR),
            (FsError::InvalidFd, EBADF),
            (FsError::NotEmpty, ENOTEMPTY),
            (FsError::Other("other".to_owned()), EINVAL),
        ];

        for (error, errno) in cases {
            assert_eq!(errno_for_fs(&error), errno);
        }
    }
}
