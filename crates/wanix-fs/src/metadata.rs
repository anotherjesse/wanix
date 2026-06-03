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
}

impl Metadata {
    /// Creates metadata from a file type, byte length, and Unix-style mode.
    #[must_use]
    pub fn new(file_type: FileType, len: u64, mode: u32) -> Self {
        Self {
            file_type,
            len,
            mode,
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
    use super::{DirEntry, FileType, Metadata};

    #[test]
    fn metadata_and_entries_expose_contract_fields() {
        let metadata = Metadata::new(FileType::File, 4, 0o644);
        let entry = DirEntry::new("file.txt", metadata.clone());

        assert_eq!(entry.name(), "file.txt");
        assert_eq!(entry.metadata(), &metadata);
        assert_eq!(metadata.file_type(), FileType::File);
        assert_eq!(metadata.len(), 4);
        assert_eq!(metadata.mode(), 0o644);
        assert!(!metadata.is_empty());
    }
}
