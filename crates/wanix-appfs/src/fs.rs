//! [`AppFs`]: one principal's view of the file2chan adapter.
//!
//! The view implements [`FileSystem`] over the declared [`crate::AppTree`]:
//! the root and `who` are host-answered, stream paths register host-owned
//! subscriptions on open, and guest paths route `read`/`write`/`readdir`/
//! `stat` to the guest with this view's principal stamped into every event.
//! The `FileSystem` itself never sees a caller-claimed identity.

use std::sync::Arc;

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

use crate::files::{BytesFile, GuestFile, StreamFile, require_read_only};
use crate::protocol::AppOp;
use crate::service::Shared;
use crate::tree::{AppPath, WHO_FILE};

pub(crate) mod modes {
    pub(crate) const DIRECTORY: u32 = 0o555;
    pub(crate) const READ_FILE: u32 = 0o444;
    pub(crate) const STREAM_FILE: u32 = 0o444;
    pub(crate) const GUEST_FILE: u32 = 0o666;
}

pub(crate) fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, modes::DIRECTORY)
}

pub(crate) fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

/// One principal's filesystem view of an [`crate::AppFsService`].
pub struct AppFs {
    shared: Arc<Shared>,
    principal: String,
}

impl AppFs {
    pub(crate) fn new(shared: Arc<Shared>, principal: String) -> Self {
        Self { shared, principal }
    }

    /// The principal this view acts as (opaque; from the transport layer).
    #[must_use]
    pub fn principal(&self) -> &str {
        &self.principal
    }
}

impl FileSystem for AppFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        match self.shared.tree().resolve(path)? {
            AppPath::Root => Err(FsError::IsDirectory),
            AppPath::Who => {
                require_read_only(options)?;
                Ok(Box::new(BytesFile::new(
                    self.shared.who_snapshot()?,
                    modes::READ_FILE,
                )))
            }
            AppPath::Stream(name) => {
                require_read_only(options)?;
                let (subscription, buffer) = self.shared.subscribe(name, &self.principal)?;
                Ok(Box::new(StreamFile::new(
                    Arc::clone(&self.shared),
                    subscription,
                    buffer,
                )))
            }
            AppPath::Guest(name) => {
                if !options.read && !options.write {
                    return Err(FsError::PermissionDenied);
                }
                // Declared guest paths exist by declaration, so the open is
                // local; the guest is consulted per discrete read/write.
                Ok(Box::new(GuestFile::new(
                    Arc::clone(&self.shared),
                    name.to_owned(),
                    self.principal.clone(),
                    options,
                )))
            }
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match self.shared.tree().resolve(path)? {
            AppPath::Root => Ok(directory_metadata()),
            AppPath::Who => Ok(file_metadata(
                self.shared.who_snapshot()?.len() as u64,
                modes::READ_FILE,
            )),
            AppPath::Stream(_) => Ok(file_metadata(0, modes::STREAM_FILE)),
            AppPath::Guest(name) => {
                let ok = self
                    .shared
                    .transact(AppOp::Stat, name, &self.principal, None, None)?;
                Ok(file_metadata(ok.size.unwrap_or(0), modes::GUEST_FILE))
            }
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match self.shared.tree().resolve(path)? {
            AppPath::Root => {
                let tree = self.shared.tree();
                let mut entries: Vec<DirEntry> = tree
                    .guest_files()
                    .map(|name| DirEntry::new(name, file_metadata(0, modes::GUEST_FILE)))
                    .chain(
                        tree.streams()
                            .map(|name| DirEntry::new(name, file_metadata(0, modes::STREAM_FILE))),
                    )
                    .collect();
                entries.push(DirEntry::new(WHO_FILE, file_metadata(0, modes::READ_FILE)));
                entries.sort_by(|a, b| a.name().cmp(b.name()));
                Ok(entries)
            }
            AppPath::Guest(name) => {
                let ok = self
                    .shared
                    .transact(AppOp::Readdir, name, &self.principal, None, None)?;
                let entries = ok.entries.unwrap_or_default();
                Ok(entries
                    .into_iter()
                    .map(|entry| {
                        let metadata = if entry.dir {
                            directory_metadata()
                        } else {
                            file_metadata(0, modes::GUEST_FILE)
                        };
                        DirEntry::new(entry.name, metadata)
                    })
                    .collect())
            }
            AppPath::Who | AppPath::Stream(_) => Err(FsError::NotDirectory),
        }
    }
}
