//! Client-side errno-to-[`FsError`] mapping and the crate error type.
//!
//! A 9P2000.L server reports failures as `Rlerror` frames carrying a Linux
//! errno. The mapping back into [`FsError`] is intentionally lossy: many errnos
//! collapse onto [`FsError::Other`]. This is the client's own canonical table,
//! the inverse of the server's `errno_for_fs`, kept here because the server
//! mapping is non-injective and cannot be shared.

use std::fmt;

use wanix_fs::FsError;
use wanix_protocol::P9Error;

/// Linux `ENOENT`: no such file or directory.
pub const ENOENT: u32 = 2;
/// Linux `EBADF`: bad file descriptor.
pub const EBADF: u32 = 9;
/// Linux `EACCES`: permission denied.
pub const EACCES: u32 = 13;
/// Linux `EEXIST`: file already exists.
pub const EEXIST: u32 = 17;
/// Linux `ENOTDIR`: not a directory.
pub const ENOTDIR: u32 = 20;
/// Linux `EISDIR`: is a directory.
pub const EISDIR: u32 = 21;
/// Linux `EINVAL`: invalid argument.
pub const EINVAL: u32 = 22;
/// Linux `ENOTEMPTY`: directory not empty.
pub const ENOTEMPTY: u32 = 39;
/// Linux `ENODATA`: the named extended attribute does not exist.
///
/// A `Txattrwalk("cas.hash")` returns this when the file offers no
/// content-addressed hash, signalling the client to fall back to a plain
/// `Tread` loop rather than offloading to the blob plane.
pub const ENODATA: u32 = 61;
/// Linux `ENOSYS`: function not implemented.
pub const ENOSYS: u32 = 95;

/// Maps a Linux errno reported by an `Rlerror` server reply onto an [`FsError`].
///
/// The mapping is lossy: errnos without a dedicated [`FsError`] variant collapse
/// onto [`FsError::Other`] with the numeric code preserved in the message.
#[must_use]
pub fn fs_error_for_errno(errno: u32) -> FsError {
    match errno {
        ENOENT => FsError::NotFound,
        EACCES => FsError::PermissionDenied,
        EISDIR => FsError::IsDirectory,
        ENOTDIR => FsError::NotDirectory,
        EEXIST => FsError::AlreadyExists,
        ENOTEMPTY => FsError::NotEmpty,
        EBADF => FsError::InvalidFd,
        ENOSYS => FsError::NotSupported,
        EINVAL => FsError::Other(format!("remote errno {errno}")),
        other => FsError::Other(format!("remote errno {other}")),
    }
}

/// Failure modes of a 9P client connection.
///
/// Transport, protocol-decode, and `Rlerror` failures are distinct so callers
/// can tell a broken connection from a routine filesystem error. Every variant
/// lowers into [`FsError`] for the [`wanix_fs::FileSystem`] surface.
#[derive(Debug)]
pub enum ClientError {
    /// The underlying duplex transport failed to read or write bytes.
    Io(std::io::Error),
    /// A frame failed to encode or decode against the 9P wire contract.
    Protocol(P9Error),
    /// The server returned an `Rlerror` frame with a Linux errno.
    Remote {
        /// Linux errno reported by the server.
        errno: u32,
    },
    /// The connection was poisoned by a protocol violation and is unusable.
    ///
    /// Once poisoned, every later request fails fast instead of trusting a
    /// desynchronized byte stream.
    Poisoned(String),
    /// The caller's request was rejected locally before reaching the server.
    ///
    /// Carries the [`FsError`] that should surface to the filesystem caller, for
    /// example mutating the filesystem root, which has no parent directory.
    Request(FsError),
}

impl ClientError {
    /// Lowers this error into the [`FsError`] surface used by [`wanix_fs`].
    #[must_use]
    pub fn into_fs_error(self) -> FsError {
        match self {
            Self::Io(error) => FsError::Other(format!("9P client I/O error: {error}")),
            Self::Protocol(error) => FsError::Other(format!("9P client protocol error: {error}")),
            Self::Remote { errno } => fs_error_for_errno(errno),
            Self::Poisoned(reason) => FsError::Other(format!("9P connection poisoned: {reason}")),
            Self::Request(error) => error,
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "9P client I/O error: {error}"),
            Self::Protocol(error) => write!(f, "9P client protocol error: {error}"),
            Self::Remote { errno } => write!(f, "9P server returned errno {errno}"),
            Self::Poisoned(reason) => write!(f, "9P connection poisoned: {reason}"),
            Self::Request(error) => write!(f, "9P client request error: {error}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Protocol(error) => Some(error),
            Self::Request(error) => Some(error),
            Self::Remote { .. } | Self::Poisoned(_) => None,
        }
    }
}

impl From<FsError> for ClientError {
    fn from(error: FsError) -> Self {
        Self::Request(error)
    }
}

impl From<std::io::Error> for ClientError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<P9Error> for ClientError {
    fn from(error: P9Error) -> Self {
        Self::Protocol(error)
    }
}

impl From<ClientError> for FsError {
    fn from(error: ClientError) -> Self {
        error.into_fs_error()
    }
}

/// Result type for fallible 9P client operations.
pub type ClientResult<T> = Result<T, ClientError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_errnos_map_to_dedicated_variants() {
        assert_eq!(fs_error_for_errno(ENOENT), FsError::NotFound);
        assert_eq!(fs_error_for_errno(EACCES), FsError::PermissionDenied);
        assert_eq!(fs_error_for_errno(EISDIR), FsError::IsDirectory);
        assert_eq!(fs_error_for_errno(ENOTDIR), FsError::NotDirectory);
        assert_eq!(fs_error_for_errno(EEXIST), FsError::AlreadyExists);
        assert_eq!(fs_error_for_errno(ENOTEMPTY), FsError::NotEmpty);
        assert_eq!(fs_error_for_errno(EBADF), FsError::InvalidFd);
        assert_eq!(fs_error_for_errno(ENOSYS), FsError::NotSupported);
    }

    #[test]
    fn unknown_errno_collapses_to_other() {
        assert!(matches!(fs_error_for_errno(12345), FsError::Other(_)));
        assert!(matches!(fs_error_for_errno(EINVAL), FsError::Other(_)));
    }

    #[test]
    fn remote_error_lowers_through_errno_table() {
        let error = ClientError::Remote { errno: ENOENT };
        assert_eq!(error.into_fs_error(), FsError::NotFound);
    }

    #[test]
    fn poisoned_error_lowers_to_other() {
        let error = ClientError::Poisoned("desync".to_owned());
        assert!(matches!(error.into_fs_error(), FsError::Other(_)));
    }
}
