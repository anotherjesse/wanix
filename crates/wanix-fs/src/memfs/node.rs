use crate::{FileType, Metadata, MetadataTimes};

pub(super) const DEFAULT_DIR_MODE: u32 = 0o755;
pub(super) const DEFAULT_FILE_MODE: u32 = 0o644;
pub(super) const PERMISSION_MODE_MASK: u32 = 0o7777;

#[derive(Debug, Clone)]
pub(super) struct Node {
    pub(super) kind: FileType,
    pub(super) data: Vec<u8>,
    mode: u32,
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

impl Node {
    pub(super) fn dir(mode: u32) -> Self {
        Self {
            kind: FileType::Directory,
            mode,
            data: Vec::new(),
            accessed_time_ns: 0,
            modified_time_ns: 0,
            changed_time_ns: 0,
        }
    }

    pub(super) fn file(data: Vec<u8>, mode: u32) -> Self {
        Self {
            kind: FileType::File,
            mode,
            data,
            accessed_time_ns: 0,
            modified_time_ns: 0,
            changed_time_ns: 0,
        }
    }

    pub(super) fn metadata(&self, len: u64) -> Metadata {
        let times = MetadataTimes::new(
            self.accessed_time_ns,
            self.modified_time_ns,
            self.changed_time_ns,
        );
        Metadata::new_with_times(self.kind, len, self.mode, times)
    }

    pub(super) fn is_directory(&self) -> bool {
        self.kind == FileType::Directory
    }

    pub(super) fn set_times(&mut self, accessed_time_ns: u64, modified_time_ns: u64) {
        self.accessed_time_ns = accessed_time_ns;
        self.modified_time_ns = modified_time_ns;
    }

    pub(super) fn set_permissions(&mut self, mode: u32) {
        self.mode = mode;
    }
}
