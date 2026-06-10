//! The synchronous native-wire server dispatcher.
//!
//! [`serve_one`] runs one stream to completion against an exported
//! `dyn FileSystem`. It reads exactly one [`Inbound`] frame and dispatches:
//!
//! - [`Inbound::OneShot`] → call the trait method, write one [`FsResponse`],
//!   return (the caller drops the stream).
//! - [`Inbound::Open`] → call [`FileSystem::open`], write one [`OpenResponse`],
//!   then run the open-file [`FileOp`]/[`FileReply`] loop until the client sends
//!   [`FileOp::Close`] or half-closes its send side (a clean EOF on the read),
//!   then return.
//!
//! Three invariants from the design plan (§7, risk register) are enforced here:
//!
//! - **No deadline on the idle `FileOp` read.** The server's read of the next
//!   `FileOp` is the idle wait of a live subscription (e.g. `#agent/events`); a
//!   deadline there would tear down a healthy session. The `deadline` argument is
//!   threaded for the transport to apply to *in-flight writes only*; the wire
//!   crate itself never times the read. On the in-memory Phase-2 path it is
//!   simply unused.
//! - **`Chunk` cap independent of the client `Read{max}`.** Each read chunk is
//!   clamped to `min(max, MAX_CHUNK_LEN, iounit_hint)` so a client looping `Read`
//!   with a huge `max` cannot force a large server buffer.
//! - **Streaming rides the one stream.** A never-EOF device read parks only this
//!   stream's blocking read; siblings are independent streams. No dedicated
//!   stream machinery (`StreamingImportFs`) is needed.

use std::io::{ErrorKind, Read, Write};
use std::sync::Arc;
use std::time::Duration;

use wanix_fs::{
    File, FileSeekFrom, FileSystem, FsResult, MetadataLookup, NormalizedPath, OpenOptions,
};

use crate::error::WireFsError;
use crate::frame::{FrameError, MAX_CHUNK_LEN, MAX_FRAME_LEN, read_frame, write_frame};
use crate::proto::{
    FileOp, FileReply, FsRequest, FsResponse, Inbound, OpenOk, OpenRequest, OpenResponse,
    ReadDirPage,
};
use crate::value::{WireDirEntry, WireMetadata, WireSeek};

/// Serves exactly one native-wire stream against `root` until it completes.
///
/// Reads one [`Inbound`], dispatches it, and (for an open) runs the open-file
/// loop. Returns once the one-shot reply is written or the open-file stream is
/// closed/half-closed. Any framing or transport error ends the stream silently
/// (the peer observes the dropped stream); an *application* [`wanix_fs::FsError`]
/// is reported in-band as a [`WireFsError`].
///
/// `deadline` is threaded for the transport to bound in-flight writes; it is
/// never applied to the idle `FileOp` read (see the module docs).
pub fn serve_one<S: Read + Write>(
    root: &Arc<dyn FileSystem>,
    mut stream: S,
    deadline: Option<Duration>,
) {
    let _ = deadline; // Applied by the transport (BlockingDuplex) to writes only.
    let inbound: Inbound = match read_frame(&mut stream, MAX_FRAME_LEN) {
        Ok(inbound) => inbound,
        // A clean EOF or transport fault before the first frame: nothing to do.
        Err(_) => return,
    };
    match inbound {
        Inbound::OneShot(request) => {
            let response = dispatch_one_shot(root.as_ref(), request);
            let _ = write_frame(&mut stream, &response, MAX_FRAME_LEN);
        }
        Inbound::Open(request) => serve_open(root.as_ref(), &mut stream, request),
    }
}

/// Parses a wire path string into a [`NormalizedPath`], surfacing a bad path as
/// the typed [`WireFsError::InvalidPath`] rather than a transport fault.
fn parse_path(path: &str) -> Result<NormalizedPath, WireFsError> {
    NormalizedPath::new(path).map_err(WireFsError::from)
}

/// Dispatches one [`FsRequest`] to the backing filesystem and builds its reply.
fn dispatch_one_shot(root: &dyn FileSystem, request: FsRequest) -> FsResponse {
    match request {
        FsRequest::Stat {
            path,
            follow_symlink,
        } => FsResponse::Stat(stat(root, &path, follow_symlink)),
        FsRequest::ReadDir { path, cookie } => FsResponse::ReadDir(read_dir(root, &path, cookie)),
        FsRequest::ReadLink { path } => {
            FsResponse::ReadLink(unit_path(&path, |p| root.read_link(p)))
        }
        FsRequest::Symlink { target, path } => {
            FsResponse::Unit(unit_path(&path, |p| root.symlink(&target, p)))
        }
        FsRequest::HardLink { old_path, new_path } => {
            FsResponse::Unit(two_path(&old_path, &new_path, |a, b| root.hard_link(a, b)))
        }
        FsRequest::CreateDir { path } => FsResponse::Unit(unit_path(&path, |p| root.create_dir(p))),
        FsRequest::Remove { path, dir } => FsResponse::Unit(unit_path(&path, |p| {
            if dir {
                root.remove_dir(p)
            } else {
                root.remove_file(p)
            }
        })),
        FsRequest::Rename { old_path, new_path } => {
            FsResponse::Unit(two_path(&old_path, &new_path, |a, b| root.rename(a, b)))
        }
        FsRequest::SetPermissions { path, permissions } => {
            FsResponse::Unit(unit_path(&path, |p| root.set_permissions(p, permissions)))
        }
        FsRequest::SetTimes {
            path,
            accessed_time_ns,
            modified_time_ns,
        } => FsResponse::Unit(unit_path(&path, |p| {
            root.set_times(p, accessed_time_ns, modified_time_ns)
        })),
        FsRequest::ContentHash { path } => FsResponse::ContentHash(content_hash(root, &path)),
    }
}

/// `metadata` / `metadata_with_lookup` in one round trip.
fn stat(
    root: &dyn FileSystem,
    path: &str,
    follow_symlink: bool,
) -> Result<WireMetadata, WireFsError> {
    let path = parse_path(path)?;
    let lookup = if follow_symlink {
        MetadataLookup::FollowSymlink
    } else {
        MetadataLookup::NoFollow
    };
    root.metadata_with_lookup(&path, lookup)
        .map(WireMetadata::from)
        .map_err(WireFsError::from)
}

/// Bulk `read_dir`. Paging is bulk-by-default: the whole listing fits in one
/// `ReadDirPage` (the `MAX_FRAME_LEN` ceiling bounds it); a continuation
/// `cookie` is accepted but a bulk reply never asks for one.
fn read_dir(
    root: &dyn FileSystem,
    path: &str,
    cookie: Option<u64>,
) -> Result<ReadDirPage, WireFsError> {
    // A continuation past the single bulk page has no further entries.
    if cookie.is_some() {
        return Ok(ReadDirPage {
            entries: Vec::new(),
            more: false,
            cookie: None,
        });
    }
    let path = parse_path(path)?;
    let entries = root.read_dir(&path).map_err(WireFsError::from)?;
    Ok(ReadDirPage {
        entries: entries.iter().map(WireDirEntry::from).collect(),
        more: false,
        cookie: None,
    })
}

/// `content_hash` → typed `Option<[u8; 32]>`.
fn content_hash(root: &dyn FileSystem, path: &str) -> Result<Option<[u8; 32]>, WireFsError> {
    let path = parse_path(path)?;
    root.content_hash(&path)
        .map(|hash| hash.map(|hash| *hash.as_bytes()))
        .map_err(WireFsError::from)
}

/// Runs a single-path unit op, mapping `FsError` to `WireFsError`.
fn unit_path<T>(
    path: &str,
    op: impl FnOnce(&NormalizedPath) -> FsResult<T>,
) -> Result<T, WireFsError> {
    let path = parse_path(path)?;
    op(&path).map_err(WireFsError::from)
}

/// Runs a two-path unit op, mapping `FsError` to `WireFsError`.
fn two_path(
    old_path: &str,
    new_path: &str,
    op: impl FnOnce(&NormalizedPath, &NormalizedPath) -> FsResult<()>,
) -> Result<(), WireFsError> {
    let old_path = parse_path(old_path)?;
    let new_path = parse_path(new_path)?;
    op(&old_path, &new_path).map_err(WireFsError::from)
}

/// Opens the requested file and runs the open-file sub-protocol on this stream.
fn serve_open<S: Read + Write>(root: &dyn FileSystem, stream: &mut S, request: OpenRequest) {
    let OpenRequest {
        path,
        options,
        append: _append,
    } = request;
    let opened = open_file(root, &path, options);
    let mut file = match opened {
        Ok((file, ok)) => {
            if write_frame(stream, &OpenResponse(Ok(ok)), MAX_FRAME_LEN).is_err() {
                return;
            }
            file
        }
        Err(error) => {
            let _ = write_frame(stream, &OpenResponse(Err(error)), MAX_FRAME_LEN);
            return;
        }
    };
    file_loop(stream, file.as_mut());
}

/// Opens `path` and builds the [`OpenOk`] facts the handle needs up front.
fn open_file(
    root: &dyn FileSystem,
    path: &str,
    options: crate::value::WireOpenOptions,
) -> Result<(Box<dyn File>, OpenOk), WireFsError> {
    let path = parse_path(path)?;
    let options: OpenOptions = options.into();
    let file = root.open(&path, options).map_err(WireFsError::from)?;
    let metadata = file.metadata().map_err(WireFsError::from)?;
    let ok = OpenOk {
        seekable: file.is_seekable(),
        iounit_hint: MAX_CHUNK_LEN as u32,
        metadata: WireMetadata::from(&metadata),
    };
    Ok((file, ok))
}

/// The open-file [`FileOp`]/[`FileReply`] loop, one frame at a time.
///
/// The read of the next `FileOp` is the idle wait (no deadline). A clean EOF on
/// the read means the client dropped its `File` (half-close) — the server tears
/// down its reader and returns. [`FileOp::Close`] ends the loop the same way.
fn file_loop<S: Read + Write>(stream: &mut S, file: &mut dyn File) {
    loop {
        let op: FileOp = match read_frame(stream, MAX_FRAME_LEN) {
            Ok(op) => op,
            Err(FrameError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                // Client dropped its send half: clean teardown.
                return;
            }
            Err(_) => return,
        };
        let reply = match op {
            FileOp::Read { max } => read_op(file, max),
            FileOp::Write(bytes) => write_op(file, &bytes),
            FileOp::Seek(seek) => seek_op(file, seek),
            FileOp::SetLen(len) => match file.set_len(len) {
                Ok(()) => FileReply::Ok,
                Err(error) => FileReply::Err(error.into()),
            },
            FileOp::Stat => match file.metadata() {
                Ok(metadata) => FileReply::Stat(WireMetadata::from(&metadata)),
                Err(error) => FileReply::Err(error.into()),
            },
            // Honest readiness crosses the wire so an importer's poll-style
            // wait (e.g. a cancellable `cat` of a never-EOF device) does not
            // degenerate into a parked blocking read.
            FileOp::ReadReady => match file.read_ready() {
                Ok(ready) => FileReply::Ready(ready),
                Err(error) => FileReply::Err(error.into()),
            },
            FileOp::Close => return,
        };
        // The reply is an in-flight write: a transport deadline (applied by the
        // BlockingDuplex) rides here, never on the idle read above.
        if write_frame(stream, &reply, MAX_FRAME_LEN).is_err() {
            return;
        }
    }
}

/// Serves one `Read`, clamping the chunk to `min(max, MAX_CHUNK_LEN)`.
///
/// The clamp keeps the server buffer bounded regardless of the client's `max`
/// (the `iounit_hint` the client received already equals `MAX_CHUNK_LEN`, so the
/// two-way clamp suffices). A 0-byte chunk is EOF for a regular file; a
/// never-EOF device simply blocks in `file.read` until it has bytes.
fn read_op(file: &mut dyn File, max: u32) -> FileReply {
    let cap = (max as usize).min(MAX_CHUNK_LEN);
    if cap == 0 {
        return FileReply::Chunk(Vec::new());
    }
    let mut buf = vec![0_u8; cap];
    match file.read(&mut buf) {
        Ok(count) => {
            buf.truncate(count);
            FileReply::Chunk(buf)
        }
        Err(error) => FileReply::Err(error.into()),
    }
}

/// Serves one `Write`.
fn write_op(file: &mut dyn File, bytes: &[u8]) -> FileReply {
    match file.write(bytes) {
        Ok(count) => FileReply::Wrote(count as u32),
        Err(error) => FileReply::Err(error.into()),
    }
}

/// Serves one server-authoritative `Seek`.
fn seek_op(file: &mut dyn File, seek: WireSeek) -> FileReply {
    let from: FileSeekFrom = seek.into();
    match file.seek(from) {
        Ok(offset) => FileReply::Seeked(offset),
        Err(error) => FileReply::Err(error.into()),
    }
}
