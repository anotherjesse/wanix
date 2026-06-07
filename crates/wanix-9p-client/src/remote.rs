//! [`RemoteFs`]: the client-side [`wanix_fs::FileSystem`] over a 9P connection.
//!
//! `RemoteFs` holds the shared [`P9Conn`] behind an `Arc<Mutex<_>>`. The server's
//! `serve_stream` is strictly serial, so the client keeps a single request
//! outstanding; concurrent [`wanix_fs::FileSystem`] callers serialize on the
//! mutex rather than racing on the wire. Each method walks from the attach root
//! to a guarded scratch fid, issues the matching 9P exchange, and lets the guard
//! clunk the fid on every exit path.

use std::sync::{Arc, Mutex};

use wanix_fs::{FsResult, Metadata, NormalizedPath, OpenOptions};
use wanix_protocol::{
    P9_QID_TYPE_DIR, p9_decode_rgetattr, p9_decode_rlcreate, p9_decode_rlopen, p9_tgetattr,
    p9_tlcreate, p9_tlopen,
};

use crate::attr::metadata_from_attr;
use crate::conn::P9Conn;
use crate::error::ClientResult;
use crate::file::RemoteFile;
use crate::open::open_flags_for;
use crate::transport::Duplex;
use crate::walk::{lock, walk_to, walkgetattr_to};

mod ops;

/// `Tgetattr` mask requesting every attribute the server can supply.
pub(crate) const GETATTR_ALL: u64 = u64::MAX;

/// 9P open-flag bit requesting append semantics (`O_APPEND`).
const O_APPEND: u32 = 0o2000;

/// Mode bits requested when creating a regular file (`0o644`).
const CREATE_FILE_MODE: u32 = 0o644;

/// Default group id for created nodes; the server owns real ownership policy.
const DEFAULT_GID: u32 = 0;

/// A client filesystem that proxies operations to a remote 9P server.
pub struct RemoteFs {
    conn: Arc<Mutex<P9Conn>>,
}

impl RemoteFs {
    /// Negotiates a session over `transport` and binds the served root.
    ///
    /// The transport is any blocking, bidirectional byte stream. Negotiation
    /// offers `9P2000.L.Google.2` so the server may enable `Twalkgetattr`
    /// batching, then attaches the root fid.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::ClientError`] when version or attach negotiation
    /// fails.
    pub fn connect(transport: Box<dyn Duplex>) -> ClientResult<Self> {
        let conn = P9Conn::connect(transport)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Negotiates a session and attaches the named subtree `aname`.
    ///
    /// Used to import a scoped capability from a grant-gated server: the server
    /// authorizes the attach by the verified peer identity *and* this `aname`,
    /// installing the matching [`wanix_vfs::SubtreeFs`] as the connection root.
    /// [`Self::connect`] uses the empty root `aname`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::ClientError`] when version or attach negotiation
    /// fails, including a default-deny `EACCES` rejection.
    pub fn connect_with_aname(transport: Box<dyn Duplex>, aname: &str) -> ClientResult<Self> {
        let conn = P9Conn::connect_with_aname(transport, aname)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Wraps an already-negotiated connection as a filesystem.
    ///
    /// Useful when a caller drove [`P9Conn::connect`] directly, for example to
    /// inspect the negotiated `msize` or Google version before mounting.
    #[must_use]
    pub fn attach(conn: P9Conn) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    /// Returns whether the connection negotiated the `Twalkgetattr` extension.
    #[must_use]
    pub fn supports_walkgetattr(&self) -> bool {
        match self.conn.lock() {
            Ok(conn) => conn.supports_walkgetattr(),
            Err(_) => false,
        }
    }

    /// Returns a clone of the shared connection handle.
    pub(crate) fn conn(&self) -> Arc<Mutex<P9Conn>> {
        Arc::clone(&self.conn)
    }

    /// Walks to `path`, fetches its attributes, and inverts them to metadata.
    ///
    /// When the server speaks the Google.2 extension the walk and attribute fetch
    /// collapse into a single `Twalkgetattr`, saving a round trip; otherwise a
    /// `Twalk` is followed by a `Tgetattr` on the resulting fid.
    pub(crate) fn metadata_impl(&self, path: &NormalizedPath) -> ClientResult<Metadata> {
        if self.supports_walkgetattr() {
            let (_guard, attr) = walkgetattr_to(&self.conn, path)?;
            return Ok(metadata_from_attr(&attr));
        }
        let guard = walk_to(&self.conn, path)?;
        let fid = guard.fid();
        let mut held = lock(&self.conn)?;
        let reply = held.rpc(|tag| Ok(p9_tgetattr(tag, fid, GETATTR_ALL)))?;
        let attr = p9_decode_rgetattr(&reply)?;
        Ok(metadata_from_attr(&attr))
    }

    /// Walks to `path`, opens or creates the fid, and builds a [`RemoteFile`].
    ///
    /// When `options.create` is set the file is created with `Tlcreate` against
    /// the walked parent fid, matching the server's create path; otherwise the
    /// existing file is opened with `Tlopen` after walking to it directly.
    ///
    /// Honest seekability: a `Tgetattr` on the open fid decides whether the file
    /// is a regular, offset-addressable file. Non-regular files (devices, service
    /// streams) are reported `is_seekable() == false`. Append is delegated to the
    /// server through `O_APPEND` so writes append without a client-side offset
    /// race.
    pub(crate) fn open_impl(
        &self,
        path: &NormalizedPath,
        options: OpenOptions,
        append: bool,
    ) -> ClientResult<RemoteFile> {
        let mut flags = open_flags_for(options);
        if append {
            flags |= O_APPEND;
        }
        let (guard, iounit) = if options.create {
            self.create_fid(path, flags)?
        } else {
            self.open_fid(path, flags)?
        };
        let fid = guard.fid();
        let seekable = self.fid_is_regular(fid)?;
        // The fid now belongs to the open handle; defuse the scratch guard so it
        // is not double-clunked, then let the handle own the clunk.
        let fid = guard.into_fid();
        Ok(RemoteFile::new(self.conn(), fid, iounit, seekable, append))
    }

    /// Walks to `path` and opens it with `Tlopen`, returning the fid and iounit.
    fn open_fid(
        &self,
        path: &NormalizedPath,
        flags: u32,
    ) -> ClientResult<(crate::fid::ScratchFid, u32)> {
        let guard = walk_to(&self.conn, path)?;
        let fid = guard.fid();
        let mut held = lock(&self.conn)?;
        let reply = held.rpc(|tag| Ok(p9_tlopen(tag, fid, flags)))?;
        let (_qid, iounit) = p9_decode_rlopen(&reply)?;
        drop(held);
        Ok((guard, iounit))
    }

    /// Walks to the parent of `path` and creates the child with `Tlcreate`.
    ///
    /// `Tlcreate` consumes the walked parent fid and rebinds it to the open child
    /// file, so the returned guard owns the open child fid.
    fn create_fid(
        &self,
        path: &NormalizedPath,
        flags: u32,
    ) -> ClientResult<(crate::fid::ScratchFid, u32)> {
        let parent = path.parent().ok_or(crate::error::ClientError::Request(
            wanix_fs::FsError::IsDirectory,
        ))?;
        let name = path.file_name().to_owned();
        let guard = walk_to(&self.conn, &parent)?;
        let fid = guard.fid();
        let mut held = lock(&self.conn)?;
        let reply =
            held.rpc(|tag| p9_tlcreate(tag, fid, &name, flags, CREATE_FILE_MODE, DEFAULT_GID))?;
        let (_qid, iounit) = p9_decode_rlcreate(&reply)?;
        drop(held);
        Ok((guard, iounit))
    }

    /// Learns `path`'s content hash over the wire via the `cas.hash` xattr.
    ///
    /// This is the client half of the control/data split: when the server's
    /// filesystem content-addresses a file, the hash crosses as a synthetic
    /// xattr read, and a CAS-aware caller fetches the blob from the data plane
    /// instead of looping `Tread`. A file with no offloadable hash returns
    /// `Ok(None)` (the server replied `ENODATA`), so the caller reads inline.
    pub(crate) fn content_hash_impl(
        &self,
        path: &NormalizedPath,
    ) -> ClientResult<Option<wanix_fs::ContentHash>> {
        crate::cas::content_hash_for(&self.conn, path)
    }

    /// Reports whether the file behind `fid` is a regular file via `Tgetattr`.
    fn fid_is_regular(&self, fid: u32) -> ClientResult<bool> {
        let mut held = lock(&self.conn)?;
        let reply = held.rpc(|tag| Ok(p9_tgetattr(tag, fid, GETATTR_ALL)))?;
        let attr = p9_decode_rgetattr(&reply)?;
        Ok(
            metadata_from_attr(&attr).file_type() == wanix_fs::FileType::File
                && attr.qid.qid_type != P9_QID_TYPE_DIR,
        )
    }
}

impl wanix_fs::FileSystem for RemoteFs {
    fn open(
        &self,
        path: &NormalizedPath,
        options: OpenOptions,
    ) -> FsResult<Box<dyn wanix_fs::File>> {
        // OpenOptions carries no append bit; appending is requested via a
        // dedicated path. Standard opens never append.
        let file = self.open_impl(path, options, false)?;
        Ok(Box::new(file))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        Ok(self.metadata_impl(path)?)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<wanix_fs::DirEntry>> {
        Ok(ops::read_dir(self, path)?)
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        Ok(ops::create_dir(self, path)?)
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        Ok(ops::remove(self, path, false)?)
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        Ok(ops::remove(self, path, true)?)
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        Ok(ops::rename(self, old_path, new_path)?)
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        Ok(ops::read_link(self, path)?)
    }

    fn content_hash(&self, path: &NormalizedPath) -> FsResult<Option<wanix_fs::ContentHash>> {
        Ok(self.content_hash_impl(path)?)
    }
}
