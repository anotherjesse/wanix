/// Broad file kind used by Wanix metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Regular byte file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
}

/// File metadata shared by directory entries and stat-like operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    file_type: FileType,
    len: u64,
    mode: u32,
    link_count: u64,
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

/// Nanosecond timestamps associated with file metadata.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MetadataTimes {
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

impl MetadataTimes {
    /// Creates metadata timestamps from nanoseconds since the Unix epoch.
    #[must_use]
    pub fn new(accessed_time_ns: u64, modified_time_ns: u64, changed_time_ns: u64) -> Self {
        Self {
            accessed_time_ns,
            modified_time_ns,
            changed_time_ns,
        }
    }
}

impl Metadata {
    /// Creates metadata from a file type, byte length, and Unix-style mode.
    #[must_use]
    pub fn new(file_type: FileType, len: u64, mode: u32) -> Self {
        Self::new_with_times(file_type, len, mode, MetadataTimes::default())
    }

    /// Creates metadata with explicit nanosecond timestamps since the Unix epoch.
    #[must_use]
    pub fn new_with_times(file_type: FileType, len: u64, mode: u32, times: MetadataTimes) -> Self {
        Self::new_with_links(file_type, len, mode, 1, times)
    }

    /// Creates metadata with explicit timestamps and link count.
    #[must_use]
    pub fn new_with_links(
        file_type: FileType,
        len: u64,
        mode: u32,
        link_count: u64,
        times: MetadataTimes,
    ) -> Self {
        Self {
            file_type,
            len,
            mode,
            link_count,
            accessed_time_ns: times.accessed_time_ns,
            modified_time_ns: times.modified_time_ns,
            changed_time_ns: times.changed_time_ns,
        }
    }

    /// Returns the broad file kind.
    #[must_use]
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the byte length.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Returns whether the file has no byte content.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the Unix-style mode bits currently associated with this file.
    #[must_use]
    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// Returns the number of filesystem links to this file when known.
    #[must_use]
    pub fn link_count(&self) -> u64 {
        self.link_count
    }

    /// Returns the last-access timestamp as nanoseconds since the Unix epoch.
    #[must_use]
    pub fn accessed_time_ns(&self) -> u64 {
        self.accessed_time_ns
    }

    /// Returns the last-modified timestamp as nanoseconds since the Unix epoch.
    #[must_use]
    pub fn modified_time_ns(&self) -> u64 {
        self.modified_time_ns
    }

    /// Returns the metadata-changed timestamp as nanoseconds since the Unix epoch.
    #[must_use]
    pub fn changed_time_ns(&self) -> u64 {
        self.changed_time_ns
    }
}

/// Directory entry returned by readdir-like operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    name: String,
    metadata: Metadata,
}

impl DirEntry {
    /// Creates a directory entry.
    #[must_use]
    pub fn new(name: impl Into<String>, metadata: Metadata) -> Self {
        Self {
            name: name.into(),
            metadata,
        }
    }

    /// Returns the entry basename.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the entry metadata.
    #[must_use]
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::{DirEntry, FileType, Metadata, MetadataTimes};

    #[test]
    fn metadata_and_entries_expose_contract_fields() {
        let metadata = Metadata::new(FileType::File, 4, 0o644);
        let entry = DirEntry::new("file.txt", metadata.clone());

        assert_eq!(entry.name(), "file.txt");
        assert_eq!(entry.metadata(), &metadata);
        assert_eq!(metadata.file_type(), FileType::File);
        assert_eq!(metadata.len(), 4);
        assert_eq!(metadata.mode(), 0o644);
        assert_eq!(metadata.link_count(), 1);
        assert_eq!(metadata.accessed_time_ns(), 0);
        assert_eq!(metadata.modified_time_ns(), 0);
        assert_eq!(metadata.changed_time_ns(), 0);
        assert!(!metadata.is_empty());

        let times = MetadataTimes::new(1, 2, 3);
        let timed = Metadata::new_with_times(FileType::File, 4, 0o644, times);
        assert_eq!(timed.link_count(), 1);
        assert_eq!(timed.accessed_time_ns(), 1);
        assert_eq!(timed.modified_time_ns(), 2);
        assert_eq!(timed.changed_time_ns(), 3);

        let linked = Metadata::new_with_links(FileType::File, 4, 0o644, 2, times);
        assert_eq!(linked.link_count(), 2);
    }
}
