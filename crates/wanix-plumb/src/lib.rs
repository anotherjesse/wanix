//! Plumber device filesystem for Rust Wanix — the `#plumb` coordination bus.
//!
//! `PlumbDevice` exposes a `#plumb` service in the Plan 9 plumber idiom: each
//! `#plumb/<topic>/send` write publishes one newline-JSON envelope
//! (`{kind,from,to,body}`) to a named topic, and each `#plumb/<topic>/recv` read
//! drains the envelopes the topic has received since the file was opened.
//! Delivery is best-effort epidemic pub/sub, **not** a durable queue: a reader
//! that was not subscribed when a message was sent never sees it, and there is
//! no acknowledgement (durable handoff belongs in `#kv` or a capsule blob).
//!
//! The transport is injected as a [`PlumbPort`]: a [`LocalPlumbPort`] delivers
//! in-process for tests and single-node use; `wanix-mesh`'s `GossipPlumbPort`
//! maps each topic to an iroh-gossip topic so a message broadcast on one node is
//! received on another. Because the device is a plain [`wanix_fs::FileSystem`],
//! it also imports for free across the mesh — `/n/A/#plumb/<topic>/recv` reads
//! node A's bus as ordinary files.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

mod buffer;
mod envelope;
mod files;
mod local;
mod path;
mod port;

pub use envelope::{MAX_ENVELOPE_LEN, PlumbEnvelope};
pub use local::LocalPlumbPort;
pub use port::{PlumbPort, PlumbStream, SharedPlumbPort};

use files::{RecvFile, SendFile, require_read_only, require_write_only};
use path::{PlumbPath, parse_path};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix plumber device filesystem";

pub(crate) mod modes {
    pub(crate) const READ_ONLY_FILE: u32 = 0o555;
    pub(crate) const STREAM_FILE: u32 = 0o666;
    pub(crate) const DIRECTORY: u32 = READ_ONLY_FILE;
}

/// Upper bound on the topic names retained for the root directory listing.
///
/// `known` is a best-effort, cosmetic record of locally touched topics (the bus
/// itself is nameless gossip). Topic names are caller-controlled — a remote 9P
/// client walking an imported `#plumb` supplies them — so this set must not grow
/// without bound: once it is full, further first-seen topics simply do not
/// appear in the listing, which has no behavioral effect (a topic still works
/// whether or not it is listed). Names are length-bounded at parse time
/// ([`path::MAX_TOPIC_LEN`]), so this caps total retained string memory.
const MAX_KNOWN_TOPICS: usize = 4096;

/// Filesystem implementing the Rust-native Wanix plumber service.
///
/// Clone-cheap: every clone shares one [`PlumbPort`] and one set of known
/// topics, so binding the device into several namespaces routes to one bus.
#[derive(Clone)]
pub struct PlumbDevice {
    port: SharedPlumbPort,
    /// Topic names that have been opened (for the root directory listing). The
    /// bus itself is nameless gossip, so the device can only list topics it has
    /// locally touched; this is a best-effort directory, not a global registry.
    known: Arc<Mutex<BTreeSet<String>>>,
}

impl fmt::Debug for PlumbDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let topics = self.known.lock().map(|t| t.len()).ok();
        f.debug_struct("PlumbDevice")
            .field("topics", &topics)
            .finish()
    }
}

impl PlumbDevice {
    /// Creates a plumber device backed by `port`.
    #[must_use]
    pub fn new(port: SharedPlumbPort) -> Self {
        Self {
            port,
            known: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    /// Creates a single-node plumber device over an in-process [`LocalPlumbPort`].
    #[must_use]
    pub fn local() -> Self {
        Self::new(Arc::new(LocalPlumbPort::new()))
    }

    /// Records `topic` as locally known so it appears in the root listing.
    ///
    /// Bounded by [`MAX_KNOWN_TOPICS`]: once the listing is full, a new
    /// first-seen topic is not recorded (it still works; it is merely absent from
    /// the cosmetic directory), so caller-controlled names cannot grow this set
    /// without limit. An already-known topic is always kept.
    fn remember(&self, topic: &str) {
        if let Ok(mut known) = self.known.lock()
            && known.len() < MAX_KNOWN_TOPICS
        {
            known.insert(topic.to_owned());
        }
    }

    fn topics(&self) -> FsResult<BTreeSet<String>> {
        self.known
            .lock()
            .map(|known| known.clone())
            .map_err(|_| FsError::Other("plumb device lock poisoned".to_owned()))
    }
}

impl FileSystem for PlumbDevice {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        match parse_path(path)? {
            PlumbPath::Root | PlumbPath::Topic(_) => Err(FsError::IsDirectory),
            PlumbPath::Send(topic) => {
                require_write_only(options)?;
                self.remember(topic);
                Ok(Box::new(SendFile::new(
                    Arc::clone(&self.port),
                    topic.to_owned(),
                )))
            }
            PlumbPath::Recv(topic) => {
                require_read_only(options)?;
                self.remember(topic);
                let stream = self.port.subscribe(topic)?;
                Ok(Box::new(RecvFile::new(stream)))
            }
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            // A topic directory exists on demand: any name is a valid topic, so
            // walking to `<topic>` always succeeds (there is no allocation step).
            PlumbPath::Root | PlumbPath::Topic(_) => Ok(directory_metadata()),
            PlumbPath::Send(_) | PlumbPath::Recv(_) => Ok(file_metadata(0, modes::STREAM_FILE)),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match parse_path(path)? {
            PlumbPath::Root => {
                let topics = self.topics()?;
                Ok(topics
                    .into_iter()
                    .map(|topic| DirEntry::new(topic, directory_metadata()))
                    .collect())
            }
            PlumbPath::Topic(_) => Ok(vec![
                DirEntry::new("recv", file_metadata(0, modes::STREAM_FILE)),
                DirEntry::new("send", file_metadata(0, modes::STREAM_FILE)),
            ]),
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
