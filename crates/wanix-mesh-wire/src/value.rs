//! `postcard`-serializable mirrors of the `wanix-fs` value types.
//!
//! [`wanix_fs::Metadata`] and [`wanix_fs::DirEntry`] have private fields and are
//! rebuilt through `Metadata::new_with_links` / `MetadataTimes::new` /
//! `DirEntry::new`. A field-order or unit (ns vs s) mismatch in a converter
//! would silently corrupt metadata with no compile error, so this is a named
//! risk in the design plan (§5, risk 4); the converters below stay in lockstep
//! with the constructor field order, and the round-trip identity tests at the
//! bottom of this module are mandatory.
//!
//! These mirrors carry no `serde` derives back into `wanix-fs`: those are leaf
//! core types and stay dependency-free, exactly as `wanix-9p/src/attr.rs`
//! mirrors `P9Attr` at the protocol edge.

use serde::{Deserialize, Serialize};
use wanix_fs::{DirEntry, FileSeekFrom, FileType, Metadata, MetadataTimes, OpenOptions};

/// Wire mirror of [`wanix_fs::FileType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireFileType {
    /// Regular byte file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
}

impl From<FileType> for WireFileType {
    fn from(file_type: FileType) -> Self {
        match file_type {
            FileType::File => Self::File,
            FileType::Directory => Self::Directory,
            FileType::Symlink => Self::Symlink,
        }
    }
}

impl From<WireFileType> for FileType {
    fn from(file_type: WireFileType) -> Self {
        match file_type {
            WireFileType::File => Self::File,
            WireFileType::Directory => Self::Directory,
            WireFileType::Symlink => Self::Symlink,
        }
    }
}

/// Wire mirror of [`wanix_fs::Metadata`].
///
/// Carries exactly the seven contract fields of `Metadata`. All three
/// timestamps are nanoseconds since the Unix epoch, matching the `*_time_ns`
/// accessors — never seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireMetadata {
    /// Broad file kind.
    pub file_type: WireFileType,
    /// Byte length.
    pub len: u64,
    /// Unix-style mode bits.
    pub mode: u32,
    /// Number of filesystem links.
    pub link_count: u64,
    /// Last-access timestamp, nanoseconds since the Unix epoch.
    pub accessed_time_ns: u64,
    /// Last-modified timestamp, nanoseconds since the Unix epoch.
    pub modified_time_ns: u64,
    /// Metadata-changed timestamp, nanoseconds since the Unix epoch.
    pub changed_time_ns: u64,
}

impl From<&Metadata> for WireMetadata {
    fn from(metadata: &Metadata) -> Self {
        Self {
            file_type: metadata.file_type().into(),
            len: metadata.len(),
            mode: metadata.mode(),
            link_count: metadata.link_count(),
            accessed_time_ns: metadata.accessed_time_ns(),
            modified_time_ns: metadata.modified_time_ns(),
            changed_time_ns: metadata.changed_time_ns(),
        }
    }
}

impl From<Metadata> for WireMetadata {
    fn from(metadata: Metadata) -> Self {
        Self::from(&metadata)
    }
}

impl From<WireMetadata> for Metadata {
    fn from(metadata: WireMetadata) -> Self {
        // Reconstruct via the public constructor; the `MetadataTimes::new`
        // argument order is (accessed, modified, changed), all in nanoseconds.
        let times = MetadataTimes::new(
            metadata.accessed_time_ns,
            metadata.modified_time_ns,
            metadata.changed_time_ns,
        );
        Metadata::new_with_links(
            metadata.file_type.into(),
            metadata.len,
            metadata.mode,
            metadata.link_count,
            times,
        )
    }
}

/// Wire mirror of [`wanix_fs::DirEntry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireDirEntry {
    /// Entry basename.
    pub name: String,
    /// Full entry metadata (no 9P placeholder-size problem on this wire).
    pub metadata: WireMetadata,
}

impl From<&DirEntry> for WireDirEntry {
    fn from(entry: &DirEntry) -> Self {
        Self {
            name: entry.name().to_owned(),
            metadata: entry.metadata().into(),
        }
    }
}

impl From<DirEntry> for WireDirEntry {
    fn from(entry: DirEntry) -> Self {
        Self::from(&entry)
    }
}

impl From<WireDirEntry> for DirEntry {
    fn from(entry: WireDirEntry) -> Self {
        DirEntry::new(entry.name, entry.metadata.into())
    }
}

/// Wire mirror of [`wanix_fs::OpenOptions`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireOpenOptions {
    /// Open for reading.
    pub read: bool,
    /// Open for writing.
    pub write: bool,
    /// Create the file if it is missing.
    pub create: bool,
    /// Truncate the file after opening.
    pub truncate: bool,
}

impl From<OpenOptions> for WireOpenOptions {
    fn from(options: OpenOptions) -> Self {
        Self {
            read: options.read,
            write: options.write,
            create: options.create,
            truncate: options.truncate,
        }
    }
}

impl From<WireOpenOptions> for OpenOptions {
    fn from(options: WireOpenOptions) -> Self {
        OpenOptions {
            read: options.read,
            write: options.write,
            create: options.create,
            truncate: options.truncate,
        }
    }
}

/// Wire mirror of [`wanix_fs::FileSeekFrom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireSeek {
    /// Seek to an absolute byte offset from the beginning of the file.
    Start(u64),
    /// Seek relative to the current file offset.
    Current(i64),
    /// Seek relative to the current end of file.
    End(i64),
}

impl From<FileSeekFrom> for WireSeek {
    fn from(from: FileSeekFrom) -> Self {
        match from {
            FileSeekFrom::Start(offset) => Self::Start(offset),
            FileSeekFrom::Current(delta) => Self::Current(delta),
            FileSeekFrom::End(delta) => Self::End(delta),
        }
    }
}

impl From<WireSeek> for FileSeekFrom {
    fn from(from: WireSeek) -> Self {
        match from {
            WireSeek::Start(offset) => Self::Start(offset),
            WireSeek::Current(delta) => Self::Current(delta),
            WireSeek::End(delta) => Self::End(delta),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WireDirEntry, WireMetadata, WireOpenOptions, WireSeek};
    use wanix_fs::{DirEntry, FileSeekFrom, FileType, Metadata, MetadataTimes, OpenOptions};

    /// A `Metadata` with every field set to a distinct non-default value so a
    /// dropped or swapped field would change equality.
    fn rich_metadata() -> Metadata {
        let times = MetadataTimes::new(11, 22, 33);
        Metadata::new_with_links(FileType::Symlink, 4096, 0o755, 7, times)
    }

    #[test]
    fn metadata_round_trips_through_the_wire_mirror() {
        let metadata = rich_metadata();
        let wire = WireMetadata::from(&metadata);
        let back = Metadata::from(wire);
        assert_eq!(back, metadata);
    }

    #[test]
    fn metadata_round_trips_for_every_file_type() {
        for file_type in [FileType::File, FileType::Directory, FileType::Symlink] {
            let times = MetadataTimes::new(1, 2, 3);
            let metadata = Metadata::new_with_links(file_type, 5, 0o644, 2, times);
            assert_eq!(Metadata::from(WireMetadata::from(&metadata)), metadata);
        }
    }

    #[test]
    fn dir_entry_round_trips_through_the_wire_mirror() {
        let entry = DirEntry::new("file.txt", rich_metadata());
        let wire = WireDirEntry::from(&entry);
        let back = DirEntry::from(wire);
        assert_eq!(back, entry);
    }

    #[test]
    fn open_options_round_trip() {
        for options in [
            OpenOptions::default(),
            OpenOptions::read(),
            OpenOptions::read_write(),
            OpenOptions {
                read: true,
                write: true,
                create: true,
                truncate: true,
            },
        ] {
            assert_eq!(OpenOptions::from(WireOpenOptions::from(options)), options);
        }
    }

    #[test]
    fn seek_round_trips_for_every_whence() {
        for from in [
            FileSeekFrom::Start(42),
            FileSeekFrom::Current(-7),
            FileSeekFrom::End(13),
        ] {
            assert_eq!(FileSeekFrom::from(WireSeek::from(from)), from);
        }
    }

    #[test]
    fn value_mirrors_survive_postcard() {
        let metadata = rich_metadata();
        let wire = WireMetadata::from(&metadata);
        let bytes = postcard::to_allocvec(&wire).expect("encode WireMetadata");
        let decoded: WireMetadata = postcard::from_bytes(&bytes).expect("decode WireMetadata");
        assert_eq!(Metadata::from(decoded), metadata);
    }
}
