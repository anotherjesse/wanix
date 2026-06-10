//! The op-level service protocol enums carried as `postcard` frames.
//!
//! The wire surface is the [`wanix_fs::FileSystem`] + [`wanix_fs::File`] traits
//! only. Every frame is a length-prefixed `postcard` body (see [`crate::frame`]).
//! There are two stream kinds (§3 of `docs/design/native-mesh-wire.md`):
//!
//! - **One-shot op stream**: the dialer writes one [`Inbound::OneShot`] carrying
//!   an [`FsRequest`], the server replies with exactly one [`FsResponse`], and
//!   the stream is dropped.
//! - **Open-file stream**: the dialer writes one [`Inbound::Open`] carrying an
//!   [`OpenRequest`]; the server replies with one [`OpenResponse`]; then the
//!   stream carries a duplex sub-protocol of [`FileOp`] (client→server) and
//!   [`FileReply`] (server→client) frames until the client half-closes (its
//!   [`crate::client::NativeFile`] `Drop`) or sends [`FileOp::Close`].
//!
//! The first frame on a stream is always an [`Inbound`], so the server reads one
//! frame and dispatches on its variant. There are **no tags** (the stream is the
//! transaction) and **no `msize`** (QUIC flow control bounds in-flight memory;
//! [`OpenOk::iounit_hint`] is only an advisory chunk hint).

use serde::{Deserialize, Serialize};

use crate::error::WireFsError;
use crate::value::{WireDirEntry, WireMetadata, WireOpenOptions, WireSeek};

/// The first frame on any native-wire stream, selecting the stream kind.
///
/// The server reads exactly one `Inbound` and dispatches: [`Self::OneShot`] is a
/// request/response/teardown op; [`Self::Open`] opens the stateful open-file
/// sub-protocol for the lifetime of one [`wanix_fs::File`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Inbound {
    /// A non-`open` `FileSystem` method: one request, one [`FsResponse`].
    OneShot(FsRequest),
    /// `open()`: opens a dedicated open-file stream (see [`OpenRequest`]).
    Open(OpenRequest),
}

/// A one-shot (non-`open`) [`wanix_fs::FileSystem`] method, encoded for the wire.
///
/// Each variant maps to exactly one trait method (§4 of the design plan). The
/// native wire carries **all** methods, including the ones the 9P bridge does not
/// (`Symlink`, `HardLink`, `SetPermissions`, `SetTimes`), so it is a faithful
/// `FileSystem` encoding rather than the 9P-bridged subset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsRequest {
    /// `metadata` / `metadata_with_lookup`: resolve `path` and stat it in one
    /// round trip. `follow_symlink` is `false` for `MetadataLookup::NoFollow`.
    Stat {
        /// Path to stat.
        path: String,
        /// Whether a final symlink component is followed to its target.
        follow_symlink: bool,
    },
    /// `read_dir`: bulk directory listing, optionally paged via `cookie`.
    ReadDir {
        /// Directory path.
        path: String,
        /// Continuation cookie from a prior [`ReadDirPage`], or `None` to start.
        cookie: Option<u64>,
    },
    /// `read_link`: the uninterpreted symlink target bytes.
    ReadLink {
        /// Symlink path.
        path: String,
    },
    /// `symlink`: create a symlink at `path` pointing at `target`.
    Symlink {
        /// Uninterpreted target bytes.
        target: Vec<u8>,
        /// Path of the new symlink.
        path: String,
    },
    /// `hard_link`: link `new_path` to the existing file at `old_path`.
    HardLink {
        /// Existing file path.
        old_path: String,
        /// New link path.
        new_path: String,
    },
    /// `create_dir`: create one directory at `path`.
    CreateDir {
        /// Directory path to create.
        path: String,
    },
    /// `remove_file` (`dir == false`) / `remove_dir` (`dir == true`).
    Remove {
        /// Path to remove.
        path: String,
        /// Whether the target is a directory.
        dir: bool,
    },
    /// `rename`: move `old_path` to `new_path`.
    Rename {
        /// Source path.
        old_path: String,
        /// Destination path.
        new_path: String,
    },
    /// `set_permissions`: set Unix-style permission bits.
    SetPermissions {
        /// Target path.
        path: String,
        /// New permission bits.
        permissions: u32,
    },
    /// `set_times`: set access and modification timestamps (nanoseconds).
    SetTimes {
        /// Target path.
        path: String,
        /// New access time, nanoseconds since the Unix epoch.
        accessed_time_ns: u64,
        /// New modification time, nanoseconds since the Unix epoch.
        modified_time_ns: u64,
    },
    /// `content_hash`: the typed `Option<ContentHash>` for `path`.
    ContentHash {
        /// Target path.
        path: String,
    },
}

/// The single reply to a one-shot [`FsRequest`].
///
/// The variant is chosen by the request's return shape; every payload is a
/// `Result<T, WireFsError>` so a typed application error crosses the wire
/// losslessly (a transport fault is a separate surface — see [`WireFsError`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsResponse {
    /// Reply to a method returning `()` (`Symlink`, `HardLink`, `CreateDir`,
    /// `Remove`, `Rename`, `SetPermissions`, `SetTimes`).
    Unit(Result<(), WireFsError>),
    /// Reply to `Stat`.
    Stat(Result<WireMetadata, WireFsError>),
    /// Reply to `ReadDir`.
    ReadDir(Result<ReadDirPage, WireFsError>),
    /// Reply to `ReadLink`.
    ReadLink(Result<Vec<u8>, WireFsError>),
    /// Reply to `ContentHash` (`None` == resolvable but not content-addressed).
    ContentHash(Result<Option<[u8; 32]>, WireFsError>),
}

/// One page of a bulk `read_dir` listing.
///
/// Each entry carries **full** [`WireMetadata`] (no 9P placeholder-size problem).
/// `more` is `true` when the server has further entries; the client then sends a
/// follow-up [`FsRequest::ReadDir`] with `cookie` on the same stream so a
/// multi-million-entry directory pages in `MAX_FRAME_LEN`-bounded frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadDirPage {
    /// Entries in this page.
    pub entries: Vec<WireDirEntry>,
    /// Whether more entries remain (continue with [`Self::cookie`]).
    pub more: bool,
    /// Continuation cookie to pass back in the next [`FsRequest::ReadDir`].
    pub cookie: Option<u64>,
}

/// The frame that opens a dedicated open-file stream.
///
/// `append` is the extra flag the 9P client carries (`open_impl(path, opts,
/// append)`); the current [`crate::client::NativeFs::open`] passes `false`,
/// matching `RemoteFs::open`. A future append-aware path sets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    /// Path to open.
    pub path: String,
    /// Open options.
    pub options: WireOpenOptions,
    /// Whether the handle appends server-side on each write.
    pub append: bool,
}

/// The single reply to an [`OpenRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenResponse(pub Result<OpenOk, WireFsError>);

/// The successful open facts, learned up front so the handle knows its lifecycle
/// without a second round trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenOk {
    /// Whether the file is seekable (a regular, offset-addressable file). A
    /// device/service stream reports `false` and refuses `seek`.
    pub seekable: bool,
    /// Advisory per-chunk transfer hint; the server still clamps each `Chunk` to
    /// `min(client_max, MAX_CHUNK_LEN, iounit_hint)`.
    pub iounit_hint: u32,
    /// Initial metadata for the opened file.
    pub metadata: WireMetadata,
}

/// A client→server operation on an open file's own stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileOp {
    /// Read up to `max` bytes; the server replies [`FileReply::Chunk`] (a
    /// 0-length `Chunk` is EOF for regular files) or [`FileReply::Eof`].
    Read {
        /// Maximum bytes the client wants (the server clamps the actual chunk).
        max: u32,
    },
    /// Write the given bytes; the server replies [`FileReply::Wrote`].
    Write(Vec<u8>),
    /// Server-authoritative seek; the server replies [`FileReply::Seeked`].
    Seek(WireSeek),
    /// Truncate/extend to `len`; the server replies [`FileReply::Ok`].
    SetLen(u64),
    /// Stat the open handle; the server replies [`FileReply::Stat`].
    Stat,
    /// Explicit handle close. Dropping the client `File` half-closes the send
    /// side instead, which the server also observes as teardown.
    Close,
    /// Non-blocking read-readiness probe; the server replies
    /// [`FileReply::Ready`] with the file's honest `read_ready()`. This is
    /// what makes a never-EOF device read *cancellable* through a `poll`-style
    /// wait on the importing side: without it the client would have to fall
    /// back to always-ready and park in a blocking `Read` round trip.
    /// Appended last so existing postcard variant indices stay stable.
    ReadReady,
}

/// A server→client reply on an open file's own stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileReply {
    /// Read data. A 0-length `Chunk` is EOF for a regular file.
    Chunk(Vec<u8>),
    /// Explicit device close for a never-EOF stream (real device close).
    Eof,
    /// Bytes written by a [`FileOp::Write`].
    Wrote(u32),
    /// Resulting absolute offset of a [`FileOp::Seek`].
    Seeked(u64),
    /// Success for a [`FileOp::SetLen`] (or other unit op).
    Ok,
    /// Reply to a [`FileOp::Stat`].
    Stat(WireMetadata),
    /// A typed application error for the preceding op.
    Err(WireFsError),
    /// Reply to a [`FileOp::ReadReady`] probe. Appended last so existing
    /// postcard variant indices stay stable.
    Ready(bool),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{MAX_FRAME_LEN, read_frame, write_frame};
    use std::io::Cursor;

    /// Encodes then decodes any frame value, asserting `postcard` round-trips it.
    fn round_trip<T>(value: &T) -> T
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let mut buf = Vec::new();
        write_frame(&mut buf, value, MAX_FRAME_LEN).expect("encode");
        let mut cursor = Cursor::new(buf);
        let decoded: T = read_frame(&mut cursor, MAX_FRAME_LEN).expect("decode");
        assert_eq!(&decoded, value);
        decoded
    }

    #[test]
    fn inbound_one_shot_and_open_round_trip() {
        round_trip(&Inbound::OneShot(FsRequest::Stat {
            path: "a/b".to_owned(),
            follow_symlink: true,
        }));
        round_trip(&Inbound::Open(OpenRequest {
            path: "dev/log".to_owned(),
            options: WireOpenOptions {
                read: true,
                ..WireOpenOptions::default()
            },
            append: false,
        }));
    }

    #[test]
    fn every_fs_request_variant_round_trips() {
        let requests = [
            FsRequest::Stat {
                path: "p".to_owned(),
                follow_symlink: false,
            },
            FsRequest::ReadDir {
                path: "d".to_owned(),
                cookie: Some(7),
            },
            FsRequest::ReadLink {
                path: "l".to_owned(),
            },
            FsRequest::Symlink {
                target: b"t".to_vec(),
                path: "s".to_owned(),
            },
            FsRequest::HardLink {
                old_path: "o".to_owned(),
                new_path: "n".to_owned(),
            },
            FsRequest::CreateDir {
                path: "d2".to_owned(),
            },
            FsRequest::Remove {
                path: "r".to_owned(),
                dir: true,
            },
            FsRequest::Rename {
                old_path: "o2".to_owned(),
                new_path: "n2".to_owned(),
            },
            FsRequest::SetPermissions {
                path: "p2".to_owned(),
                permissions: 0o644,
            },
            FsRequest::SetTimes {
                path: "p3".to_owned(),
                accessed_time_ns: 1,
                modified_time_ns: 2,
            },
            FsRequest::ContentHash {
                path: "h".to_owned(),
            },
        ];
        for request in requests {
            round_trip(&request);
        }
    }

    #[test]
    fn fs_responses_round_trip_ok_and_err() {
        round_trip(&FsResponse::Unit(Ok(())));
        round_trip(&FsResponse::Unit(Err(WireFsError::NotEmpty)));
        round_trip(&FsResponse::ReadLink(Ok(b"target".to_vec())));
        round_trip(&FsResponse::ContentHash(Ok(Some([9_u8; 32]))));
        round_trip(&FsResponse::ContentHash(Ok(None)));
        round_trip(&FsResponse::ReadDir(Ok(ReadDirPage {
            entries: Vec::new(),
            more: true,
            cookie: Some(3),
        })));
        round_trip(&FsResponse::Stat(Err(WireFsError::InvalidPath(
            "a/../b".to_owned(),
        ))));
    }

    #[test]
    fn file_ops_and_replies_round_trip() {
        for op in [
            FileOp::Read { max: 4096 },
            FileOp::Write(vec![1, 2, 3]),
            FileOp::Seek(WireSeek::End(-1)),
            FileOp::SetLen(0),
            FileOp::Stat,
            FileOp::Close,
            FileOp::ReadReady,
        ] {
            round_trip(&op);
        }
        for reply in [
            FileReply::Chunk(vec![7, 8]),
            FileReply::Chunk(Vec::new()),
            FileReply::Eof,
            FileReply::Wrote(2),
            FileReply::Seeked(42),
            FileReply::Ok,
            FileReply::Err(WireFsError::NotSupported),
            FileReply::Ready(false),
        ] {
            round_trip(&reply);
        }
    }
}
