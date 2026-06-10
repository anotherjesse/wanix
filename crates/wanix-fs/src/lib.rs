//! Filesystem contracts for the Rust-native Wanix port.
//!
//! This crate owns path rules, metadata, file traits, filesystem traits,
//! errors, and the first in-memory filesystem used by namespace/task tests.

mod buffer;
mod content_hash;
mod error;
mod localfs;
mod memfs;
mod metadata;
mod path;
mod traits;
mod verbs;

pub use buffer::{DEFAULT_MAX_BUFFERED_BYTES, LineBuffer};
pub use content_hash::{CONTENT_HASH_HEX_LEN, CONTENT_HASH_LEN, ContentHash};
pub use error::{FsError, FsResult};
pub use localfs::LocalFs;
pub use memfs::MemFs;
pub use metadata::{DirEntry, FileType, Metadata, MetadataTimes};
pub use path::NormalizedPath;
pub use traits::{File, FileSeekFrom, FileSystem, MetadataLookup, OpenOptions};
pub use verbs::{MAX_VERB_FILE_BYTES, VerbBinFs};

/// Threshold in bytes above which a CAS-aware client should fetch a file's
/// bytes from the content-addressed data plane instead of looping `Tread` over
/// the 9P control plane.
///
/// Files at or below this size are cheap to read inline over a couple of
/// `msize` windows, so paying a blob round-trip (download + verify) for them is
/// pure overhead; the offload only pays off for genuinely large bulk content
/// (frozen worlds, rootfs images, module inputs). 256 KiB matches the blueprint
/// boundary for the control/data split.
pub const CONTENT_HASH_OFFLOAD_THRESHOLD: u64 = 256 * 1024;

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix filesystem contracts";

#[cfg(test)]
mod tests {
    use super::CRATE_PURPOSE;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }
}
