//! The synchronous native-wire client `FileSystem` facade.
//!
//! [`NativeFs`] is a [`wanix_fs::FileSystem`] that mirrors `RemoteFs`'s shape: it
//! holds a [`StreamFactory`] (in `wanix-mesh`, an iroh `Connection` + held
//! `Handle`) and, for each op, opens a fresh bidi stream, frames one request,
//! reads one reply, and maps the typed error. The open file handle itself
//! ([`NativeFile`](crate::NativeFile), in [`crate::file`]) owns the dedicated
//! open-file stream; `open` builds it from the [`OpenResponse`].

use wanix_fs::{
    DirEntry, File, FileSystem, FsError, FsResult, Metadata, MetadataLookup, NormalizedPath,
    OpenOptions,
};

use crate::ContentHash;
use crate::Duplex;
use crate::file::NativeFile;
use crate::frame::{MAX_FRAME_LEN, read_frame, write_frame};
use crate::proto::{FsRequest, FsResponse, Inbound, OpenRequest, OpenResponse, ReadDirPage};

/// Opens fresh bidirectional byte streams for the native wire.
///
/// One implementor lives in `wanix-mesh`, where `open_stream` opens an iroh QUIC
/// bidi stream on the held `tokio::runtime::Handle` and wraps it in the existing
/// `BlockingDuplex`. The wire crate only ever sees the sync [`Duplex`], so it
/// stays iroh/tokio-free.
pub trait StreamFactory: Send + Sync {
    /// Opens one fresh bidirectional stream for a single op or open file.
    ///
    /// # Errors
    ///
    /// Returns a transport I/O error when a new stream cannot be opened (e.g. the
    /// underlying connection is dead).
    fn open_stream(&self) -> std::io::Result<Box<dyn Duplex>>;
}

/// A sync [`FileSystem`] backed by the native mesh wire over a [`StreamFactory`].
pub struct NativeFs<F: StreamFactory> {
    factory: F,
}

impl<F: StreamFactory> NativeFs<F> {
    /// Builds a native filesystem facade over `factory`.
    #[must_use]
    pub fn new(factory: F) -> Self {
        Self { factory }
    }

    /// Opens a stream, maps a stream-open failure to a transport `FsError`.
    fn stream(&self) -> FsResult<Box<dyn Duplex>> {
        self.factory.open_stream().map_err(transport)
    }

    /// Runs one one-shot op: open a stream, write the request, read the reply.
    fn one_shot(&self, request: FsRequest) -> FsResult<FsResponse> {
        let mut stream = self.stream()?;
        write_frame(&mut stream, &Inbound::OneShot(request), MAX_FRAME_LEN)
            .map_err(|err| transport(std::io::Error::other(err)))?;
        read_frame(&mut stream, MAX_FRAME_LEN).map_err(|err| transport(std::io::Error::other(err)))
    }
}

/// Lowers a genuine transport fault to `FsError::Other` (never a typed
/// application error — those cross the wire as `WireFsError`).
pub(crate) fn transport(error: std::io::Error) -> FsError {
    FsError::Other(format!("mesh: {error}"))
}

/// A reply whose variant did not match the request: a protocol fault.
pub(crate) fn protocol_mismatch(what: &str) -> FsError {
    FsError::Other(format!("mesh: server returned an unexpected {what} reply"))
}

impl<F: StreamFactory> FileSystem for NativeFs<F> {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        let mut stream = self.stream()?;
        let request = OpenRequest {
            path: path.as_str().to_owned(),
            options: options.into(),
            append: false,
        };
        write_frame(&mut stream, &Inbound::Open(request), MAX_FRAME_LEN)
            .map_err(|err| transport(std::io::Error::other(err)))?;
        let OpenResponse(result) = read_frame(&mut stream, MAX_FRAME_LEN)
            .map_err(|err| transport(std::io::Error::other(err)))?;
        let ok = result.map_err(FsError::from)?;
        Ok(Box::new(NativeFile::new(stream, ok.seekable)))
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        self.metadata_with_lookup(path, MetadataLookup::FollowSymlink)
    }

    fn metadata_with_lookup(
        &self,
        path: &NormalizedPath,
        lookup: MetadataLookup,
    ) -> FsResult<Metadata> {
        match self.one_shot(FsRequest::Stat {
            path: path.as_str().to_owned(),
            follow_symlink: lookup.follow_symlinks(),
        })? {
            FsResponse::Stat(result) => result.map(Metadata::from).map_err(FsError::from),
            _ => Err(protocol_mismatch("stat")),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match self.one_shot(FsRequest::ReadDir {
            path: path.as_str().to_owned(),
            cookie: None,
        })? {
            FsResponse::ReadDir(result) => {
                let ReadDirPage { entries, .. } = result.map_err(FsError::from)?;
                Ok(entries.into_iter().map(DirEntry::from).collect())
            }
            _ => Err(protocol_mismatch("read_dir")),
        }
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        match self.one_shot(FsRequest::ReadLink {
            path: path.as_str().to_owned(),
        })? {
            FsResponse::ReadLink(result) => result.map_err(FsError::from),
            _ => Err(protocol_mismatch("read_link")),
        }
    }

    fn symlink(&self, target: &[u8], path: &NormalizedPath) -> FsResult<()> {
        self.unit(FsRequest::Symlink {
            target: target.to_vec(),
            path: path.as_str().to_owned(),
        })
    }

    fn hard_link(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        self.unit(FsRequest::HardLink {
            old_path: old_path.as_str().to_owned(),
            new_path: new_path.as_str().to_owned(),
        })
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.unit(FsRequest::CreateDir {
            path: path.as_str().to_owned(),
        })
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        self.unit(FsRequest::Remove {
            path: path.as_str().to_owned(),
            dir: false,
        })
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.unit(FsRequest::Remove {
            path: path.as_str().to_owned(),
            dir: true,
        })
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        self.unit(FsRequest::Rename {
            old_path: old_path.as_str().to_owned(),
            new_path: new_path.as_str().to_owned(),
        })
    }

    fn set_permissions(&self, path: &NormalizedPath, permissions: u32) -> FsResult<()> {
        self.unit(FsRequest::SetPermissions {
            path: path.as_str().to_owned(),
            permissions,
        })
    }

    fn set_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
    ) -> FsResult<()> {
        self.unit(FsRequest::SetTimes {
            path: path.as_str().to_owned(),
            accessed_time_ns,
            modified_time_ns,
        })
    }

    fn content_hash(&self, path: &NormalizedPath) -> FsResult<Option<ContentHash>> {
        match self.one_shot(FsRequest::ContentHash {
            path: path.as_str().to_owned(),
        })? {
            FsResponse::ContentHash(result) => result
                .map(|hash| hash.map(ContentHash::from_bytes))
                .map_err(FsError::from),
            _ => Err(protocol_mismatch("content_hash")),
        }
    }
}

impl<F: StreamFactory> NativeFs<F> {
    /// Runs a one-shot op expecting an [`FsResponse::Unit`] reply.
    fn unit(&self, request: FsRequest) -> FsResult<()> {
        match self.one_shot(request)? {
            FsResponse::Unit(result) => result.map_err(FsError::from),
            _ => Err(protocol_mismatch("unit")),
        }
    }
}
