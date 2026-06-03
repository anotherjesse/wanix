//! Filesystem contracts for the Rust-native Wanix port.
//!
//! This crate owns path rules, metadata, file traits, filesystem traits,
//! errors, and the first in-memory filesystem used by namespace/task tests.

mod error;
mod localfs;
mod memfs;
mod metadata;
mod path;
mod traits;

pub use error::{FsError, FsResult};
pub use localfs::LocalFs;
pub use memfs::MemFs;
pub use metadata::{DirEntry, FileType, Metadata};
pub use path::NormalizedPath;
pub use traits::{File, FileSeekFrom, FileSystem, MetadataLookup, OpenOptions};

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
