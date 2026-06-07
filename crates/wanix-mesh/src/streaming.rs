//! [`StreamingImportFs`]: give each blocking imported open-file its own stream.
//!
//! # The deadlock this dodges
//!
//! A [`wanix_9p_client::RemoteFs`] multiplexes every operation onto **one** 9P
//! stream behind an `Arc<Mutex<P9Conn>>`, because the server's
//! [`serve_stream`](wanix_9p::P9Server::serve_stream) is strictly serial. That is
//! correct for ordinary walk/stat/read. But an imported `#agent/<id>/events`
//! file is a *near-never-EOF blocking read*: a `Tread` on it parks the single
//! stream forever, freezing every other operation on the whole import (status,
//! prompt, a second agent, the host root). The blueprint calls this the
//! head-of-line freeze and prescribes the fix: **one QUIC bidi stream per
//! blocking open-file**, not one per attach.
//!
//! # How
//!
//! `StreamingImportFs` wraps a shared import (the everyday-ops `RemoteFs`) plus a
//! [`MeshDialer`] and the attach name. Walk, stat, readdir, mutation, and
//! ordinary file opens go to the shared import unchanged. An `open` whose path
//! matches the *blocking-stream* predicate instead **dials a fresh `RemoteFs`
//! over a new bidi stream**, opens just that one file on it, and returns a handle
//! that owns the dedicated connection — so the blocking read stalls only its own
//! stream, and dropping the file tears that stream down. Default-matched paths
//! are `#agent/<id>/events`, `#agent/<id>/reply`, and `#plumb/<topic>/recv`.

use std::sync::Arc;

use iroh::EndpointAddr;
use wanix_9p_client::RemoteFs;
use wanix_fs::{
    DirEntry, File, FileSystem, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

use crate::dialer::MeshDialer;

mod predicate;

pub use predicate::{StreamPredicate, default_blocking_stream};

/// A mesh import that routes blocking streaming opens onto dedicated bidi streams.
///
/// Holds the shared everyday-ops [`RemoteFs`], the [`MeshDialer`] and peer
/// `EndpointAddr`/`aname` needed to dial a fresh stream, and the predicate that
/// decides which opens are blocking streams. Itself a [`FileSystem`], so it binds
/// into a [`wanix_vfs::Namespace`] at `/n/<node>` exactly like a bare `RemoteFs`.
#[derive(Clone)]
pub struct StreamingImportFs {
    shared: Arc<RemoteFs>,
    dialer: MeshDialer,
    addr: EndpointAddr,
    aname: String,
    predicate: StreamPredicate,
}

impl StreamingImportFs {
    /// Wraps `shared` (the everyday-ops import of `addr`/`aname`) so blocking
    /// stream opens dial their own bidi stream via `dialer`.
    ///
    /// `predicate` selects which paths are blocking streams; pass
    /// [`default_blocking_stream`] for the standard `#agent`/`#plumb` set.
    #[must_use]
    pub fn new(
        shared: Arc<RemoteFs>,
        dialer: MeshDialer,
        addr: EndpointAddr,
        aname: impl Into<String>,
        predicate: StreamPredicate,
    ) -> Self {
        Self {
            shared,
            dialer,
            addr,
            aname: aname.into(),
            predicate,
        }
    }

    /// Opens `path` on a freshly dialed dedicated `RemoteFs`, returning a handle
    /// that owns that connection for the open file's lifetime.
    fn open_dedicated(
        &self,
        path: &NormalizedPath,
        options: OpenOptions,
    ) -> FsResult<Box<dyn File>> {
        let dedicated = self
            .dialer
            .dial_attach(self.addr.clone(), &self.aname)
            .map_err(|err| FsError::Other(format!("mesh: dial dedicated stream failed: {err}")))?;
        let file = dedicated.open(path, options)?;
        Ok(Box::new(DedicatedStreamFile {
            _connection: dedicated,
            file,
        }))
    }
}

impl FileSystem for StreamingImportFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        // A blocking stream open gets its own bidi stream; everything else rides
        // the shared connection. Only read opens are eligible — a write open is a
        // short request/response that never parks the stream.
        if options.read && !options.write && (self.predicate)(path) {
            return self.open_dedicated(path, options);
        }
        self.shared.open(path, options)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        self.shared.metadata(path)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        self.shared.read_dir(path)
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.shared.create_dir(path)
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        self.shared.remove_file(path)
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.shared.remove_dir(path)
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        self.shared.rename(old_path, new_path)
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        self.shared.read_link(path)
    }

    fn content_hash(&self, path: &NormalizedPath) -> FsResult<Option<wanix_fs::ContentHash>> {
        self.shared.content_hash(path)
    }
}

/// A file opened on its own dedicated import connection.
///
/// Owns the dedicated [`RemoteFs`] so the bidi stream lives as long as the open
/// file; dropping the handle drops both, clunking the fid and closing the stream.
struct DedicatedStreamFile {
    _connection: Arc<RemoteFs>,
    file: Box<dyn File>,
}

impl File for DedicatedStreamFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.file.read(buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.file.write(buf)
    }

    fn seek(&mut self, from: wanix_fs::FileSeekFrom) -> FsResult<u64> {
        self.file.seek(from)
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.file.read_ready()
    }

    fn write_ready(&self) -> FsResult<bool> {
        self.file.write_ready()
    }

    fn is_seekable(&self) -> bool {
        self.file.is_seekable()
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.file.metadata()
    }
}
