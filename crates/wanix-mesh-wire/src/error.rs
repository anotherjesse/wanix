//! The typed [`FsError`] mirror that crosses the native mesh wire.
//!
//! The 9P import path round-trips `FsError -> Linux errno -> FsError` lossily:
//! `InvalidPath`/`InvalidOffset`/`InvalidTime`/`Other` all collapse onto
//! `EINVAL`/`Other`, so `InvalidPath(s)` and its message are lost (see
//! `crates/wanix-9p-client/src/error.rs`). The native wire instead puts a typed
//! mirror on the wire: every op returns `Result<T, WireFsError>` encoded as
//! `postcard`, and the round trip is lossless — `InvalidPath("a/../b")` arrives
//! with its `String` intact.
//!
//! [`WireFsError`] is defined here, in `wanix-mesh-wire`, rather than by adding
//! `serde` derives to `wanix_fs::FsError`. That keeps `wanix-fs` a
//! dependency-free leaf and mirrors how `wanix-9p/src/attr.rs` maps `P9Attr` to
//! `Metadata` at the protocol edge.
//!
//! A [`WireFsError`] is an *application* error the server chose to return. A
//! genuine transport fault (dead connection, stream reset, decode failure) is a
//! separate surface and lowers to `FsError::Other` at the mesh edge, never to a
//! `WireFsError`.

use serde::{Deserialize, Serialize};
use wanix_fs::FsError;

/// A `postcard`-serializable mirror of [`wanix_fs::FsError`].
///
/// Every variant of `FsError` has exactly one counterpart here, including the
/// two that carry a `String` payload (`InvalidPath` and `Other`); the
/// [`From<FsError>`] / [`From<WireFsError>`] pair preserves those strings
/// verbatim so the conversion is lossless in both directions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireFsError {
    /// The path is not a valid normalized Wanix filesystem path.
    InvalidPath(String),
    /// The requested file or directory does not exist.
    NotFound,
    /// The operation is not supported by this file or filesystem.
    NotSupported,
    /// The caller lacks permission for this operation.
    PermissionDenied,
    /// The destination already exists.
    AlreadyExists,
    /// The path was expected to name a directory.
    NotDirectory,
    /// The path was expected to name a non-directory file.
    IsDirectory,
    /// The file descriptor is not open or is invalid.
    InvalidFd,
    /// A file offset is invalid for the requested operation.
    InvalidOffset,
    /// A file timestamp is invalid for the requested operation.
    InvalidTime,
    /// A directory removal failed because the directory has entries.
    NotEmpty,
    /// A descriptive fallback for errors that do not yet have a stable variant.
    Other(String),
}

impl From<FsError> for WireFsError {
    fn from(error: FsError) -> Self {
        match error {
            FsError::InvalidPath(path) => Self::InvalidPath(path),
            FsError::NotFound => Self::NotFound,
            FsError::NotSupported => Self::NotSupported,
            FsError::PermissionDenied => Self::PermissionDenied,
            FsError::AlreadyExists => Self::AlreadyExists,
            FsError::NotDirectory => Self::NotDirectory,
            FsError::IsDirectory => Self::IsDirectory,
            FsError::InvalidFd => Self::InvalidFd,
            FsError::InvalidOffset => Self::InvalidOffset,
            FsError::InvalidTime => Self::InvalidTime,
            FsError::NotEmpty => Self::NotEmpty,
            FsError::Other(message) => Self::Other(message),
        }
    }
}

impl From<WireFsError> for FsError {
    fn from(error: WireFsError) -> Self {
        match error {
            WireFsError::InvalidPath(path) => Self::InvalidPath(path),
            WireFsError::NotFound => Self::NotFound,
            WireFsError::NotSupported => Self::NotSupported,
            WireFsError::PermissionDenied => Self::PermissionDenied,
            WireFsError::AlreadyExists => Self::AlreadyExists,
            WireFsError::NotDirectory => Self::NotDirectory,
            WireFsError::IsDirectory => Self::IsDirectory,
            WireFsError::InvalidFd => Self::InvalidFd,
            WireFsError::InvalidOffset => Self::InvalidOffset,
            WireFsError::InvalidTime => Self::InvalidTime,
            WireFsError::NotEmpty => Self::NotEmpty,
            WireFsError::Other(message) => Self::Other(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WireFsError;
    use wanix_fs::FsError;

    /// Every `FsError` variant, including both `String`-carrying ones, so the
    /// round-trip identity test below covers the whole surface.
    fn all_fs_errors() -> Vec<FsError> {
        vec![
            FsError::InvalidPath("bad/../path".to_owned()),
            FsError::NotFound,
            FsError::NotSupported,
            FsError::PermissionDenied,
            FsError::AlreadyExists,
            FsError::NotDirectory,
            FsError::IsDirectory,
            FsError::InvalidFd,
            FsError::InvalidOffset,
            FsError::InvalidTime,
            FsError::NotEmpty,
            FsError::Other("host detail".to_owned()),
        ]
    }

    #[test]
    fn fs_error_round_trips_through_the_wire_mirror() {
        for error in all_fs_errors() {
            let wire = WireFsError::from(error.clone());
            let back = FsError::from(wire);
            assert_eq!(back, error);
        }
    }

    #[test]
    fn invalid_path_and_other_strings_are_preserved() {
        // The two payload-carrying variants are the ones the 9P errno table
        // loses; assert the message survives both conversions verbatim.
        let invalid = FsError::InvalidPath("a/../b".to_owned());
        assert_eq!(FsError::from(WireFsError::from(invalid.clone())), invalid);
        match WireFsError::from(FsError::InvalidPath("a/../b".to_owned())) {
            WireFsError::InvalidPath(path) => assert_eq!(path, "a/../b"),
            other => panic!("expected InvalidPath, got {other:?}"),
        }

        let other = FsError::Other("remote detail 42".to_owned());
        assert_eq!(FsError::from(WireFsError::from(other.clone())), other);
        match WireFsError::from(FsError::Other("remote detail 42".to_owned())) {
            WireFsError::Other(message) => assert_eq!(message, "remote detail 42"),
            unexpected => panic!("expected Other, got {unexpected:?}"),
        }
    }

    #[test]
    fn wire_mirror_round_trips_through_postcard() {
        // The mirror is only useful if it also survives the actual codec; this
        // pins that the derive encodes/decodes every variant.
        for error in all_fs_errors() {
            let wire = WireFsError::from(error.clone());
            let bytes = postcard::to_allocvec(&wire).expect("encode WireFsError");
            let decoded: WireFsError = postcard::from_bytes(&bytes).expect("decode WireFsError");
            assert_eq!(decoded, wire);
            assert_eq!(FsError::from(decoded), error);
        }
    }
}
