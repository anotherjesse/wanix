use wanix_fs::{File, FileSeekFrom, FsError, FsResult, Metadata};

use super::{file_metadata, read_from_slice, seek_offset};
use crate::{TaskId, TaskTable};

#[derive(Debug)]
pub(crate) struct NewTaskFile {
    table: TaskTable,
    parent: Option<TaskId>,
    kind: String,
    data: Option<Vec<u8>>,
    offset: usize,
}

impl NewTaskFile {
    pub(crate) fn new(table: TaskTable, parent: Option<TaskId>, kind: impl Into<String>) -> Self {
        Self {
            table,
            parent,
            kind: kind.into(),
            data: None,
            offset: 0,
        }
    }
}

impl File for NewTaskFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.data.is_none() {
            let task = match self.parent {
                Some(parent) => self.table.allocate_child(&self.kind, parent)?,
                None => self.table.allocate_root(&self.kind)?,
            };
            self.data = Some(format!("{}\n", task.id().get()).into_bytes());
        }
        read_from_slice(
            self.data.as_ref().expect("allocation populated data"),
            &mut self.offset,
            buf,
        )
    }

    fn write(&mut self, _buf: &[u8]) -> FsResult<usize> {
        Err(FsError::PermissionDenied)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, 0o555))
    }

    fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
        let len = self.data.as_ref().map_or(0, Vec::len);
        self.offset = seek_offset(self.offset, len, from)?;
        Ok(self.offset as u64)
    }

    fn tell(&self) -> FsResult<u64> {
        Ok(self.offset as u64)
    }

    fn is_seekable(&self) -> bool {
        true
    }
}
