//! In-memory pipe device filesystem for Rust Wanix.
//!
//! `PipeDevice` exposes a `#pipe` service shaped like `#term`: reading `new`
//! allocates a byte channel and returns its id, and each channel exposes `id`
//! and `data` files. Opening `<id>/data` for reading yields the read end and
//! for writing the write end; the reader observes end-of-file once every writer
//! handle has been dropped, letting two tasks compose like a Unix pipe.
//!
//! Channels are bounded by default ([`PipeCapacity::DEFAULT_BYTES`]): a write
//! into a full channel blocks until the reader drains room, which is what lets
//! concurrent pipeline stages back-pressure each other instead of buffering a
//! producer's whole output (ADR 0010 tier 2). A write after the last reader
//! end has closed fails like Unix `EPIPE` (so a producer never hangs on a
//! consumer that died mid-stream), but a write before any reader has opened
//! just buffers or blocks — consumers may attach late, so a single-threaded
//! client should not write more than one capacity of data before something is
//! draining the other end.

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

mod channel;
mod files;
mod open;
mod path;
mod state;

use channel::PipeChannel;
use open::open_pipe_path;
use path::{PipePath, parse_path};
use state::DeviceState;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix in-memory pipe device filesystem";

/// How much a pipe channel buffers before writes block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeCapacity {
    /// Writes block once this many bytes are buffered unread.
    Bounded(NonZeroUsize),
    /// Writes never block; the buffer grows without limit. Only for callers
    /// that must absorb a producer's output with no concurrent reader.
    Unbounded,
}

impl PipeCapacity {
    /// The default bound: 64 KiB, the conventional OS pipe buffer size.
    pub const DEFAULT_BYTES: usize = 64 * 1024;

    /// Returns the default bounded capacity.
    ///
    /// # Panics
    ///
    /// Never panics; [`Self::DEFAULT_BYTES`] is non-zero.
    #[must_use]
    pub fn default_bounded() -> Self {
        Self::Bounded(NonZeroUsize::new(Self::DEFAULT_BYTES).expect("default capacity is nonzero"))
    }
}

impl Default for PipeCapacity {
    fn default() -> Self {
        Self::default_bounded()
    }
}

pub(crate) mod modes {
    pub(crate) const READ_ONLY_FILE: u32 = 0o555;
    pub(crate) const STREAM_FILE: u32 = 0o666;
    pub(crate) const DIRECTORY: u32 = READ_ONLY_FILE;
}

/// Filesystem implementing the Rust-native Wanix pipe service.
#[derive(Clone)]
pub struct PipeDevice {
    capacity: PipeCapacity,
    state: Arc<Mutex<DeviceState>>,
}

impl fmt::Debug for PipeDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.state.lock() {
            Ok(state) => f
                .debug_struct("PipeDevice")
                .field("next_id", &state.next_id)
                .field("channel_count", &state.channels.len())
                .finish(),
            Err(_) => f
                .debug_struct("PipeDevice")
                .field("state", &"poisoned")
                .finish(),
        }
    }
}

impl Default for PipeDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl PipeDevice {
    /// Creates an empty pipe device with the default bounded capacity.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(PipeCapacity::default())
    }

    /// Creates an empty pipe device whose channels carry `capacity`.
    #[must_use]
    pub fn with_capacity(capacity: PipeCapacity) -> Self {
        Self {
            capacity,
            state: Arc::new(Mutex::new(DeviceState::default())),
        }
    }

    /// Allocates a channel and returns its stable id.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when device state cannot be locked.
    pub fn alloc(&self) -> FsResult<String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| FsError::Other("pipe device lock poisoned".to_owned()))?;
        state.next_id = state.next_id.saturating_add(1);
        let id = state.next_id.to_string();
        state
            .channels
            .insert(id.clone(), Arc::new(PipeChannel::new(self.capacity)));
        Ok(id)
    }

    fn channel(&self, id: &str) -> FsResult<Arc<PipeChannel>> {
        let state = self
            .state
            .lock()
            .map_err(|_| FsError::Other("pipe device lock poisoned".to_owned()))?;
        state.channels.get(id).cloned().ok_or(FsError::NotFound)
    }
}

impl FileSystem for PipeDevice {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        open_pipe_path(self, parse_path(path)?, options)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            PipePath::Root => Ok(directory_metadata()),
            PipePath::New => Ok(file_metadata(0, modes::READ_ONLY_FILE)),
            PipePath::Channel(id) => {
                self.channel(id)?;
                Ok(directory_metadata())
            }
            PipePath::Id(id) => {
                self.channel(id)?;
                Ok(file_metadata((id.len() + 1) as u64, modes::READ_ONLY_FILE))
            }
            PipePath::Data(id) => {
                self.channel(id)?;
                Ok(file_metadata(0, modes::STREAM_FILE))
            }
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match parse_path(path)? {
            PipePath::Root => {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| FsError::Other("pipe device lock poisoned".to_owned()))?;
                let mut entries = vec![DirEntry::new(
                    "new",
                    file_metadata(0, modes::READ_ONLY_FILE),
                )];
                entries.extend(
                    state
                        .channels
                        .keys()
                        .map(|id| DirEntry::new(id.clone(), directory_metadata())),
                );
                Ok(entries)
            }
            PipePath::Channel(id) => {
                self.channel(id)?;
                Ok(vec![
                    DirEntry::new("data", file_metadata(0, modes::STREAM_FILE)),
                    DirEntry::new(
                        "id",
                        file_metadata((id.len() + 1) as u64, modes::READ_ONLY_FILE),
                    ),
                ])
            }
            _ => Err(FsError::NotDirectory),
        }
    }
}

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, modes::DIRECTORY)
}

fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

#[cfg(test)]
mod tests;
