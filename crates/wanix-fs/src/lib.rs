//! Filesystem contracts for the Rust-native Wanix port.
//!
//! This crate owns path rules, metadata, file traits, filesystem traits,
//! errors, and the first in-memory filesystem used by namespace/task tests.

mod error;
mod file;
mod localfs;
mod memfs;
mod metadata;
mod path;
mod traits;

pub use error::{FsError, FsResult};
pub use file::{File, FileSeekFrom, OpenOptions};
pub use localfs::LocalFs;
pub use memfs::MemFs;
pub use metadata::{DirEntry, FileType, Metadata, MetadataTimes};
pub use path::NormalizedPath;
pub use traits::{FileSystem, MetadataLookup};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix filesystem contracts";

#[cfg(test)]
mod tests {
    use super::{CRATE_PURPOSE, File, FileSeekFrom, FileSystem, MetadataLookup, OpenOptions};

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn crate_root_reexports_core_filesystem_contracts() {
        fn accepts_file_trait<T: File + ?Sized>() {}
        fn accepts_filesystem_trait<T: FileSystem + ?Sized>() {}

        accepts_file_trait::<dyn File>();
        accepts_filesystem_trait::<dyn FileSystem>();
        assert_eq!(FileSeekFrom::Start(0), FileSeekFrom::Start(0));
        assert!(OpenOptions::read().read);
        assert!(!MetadataLookup::NoFollow.follow_symlinks());
    }
}
