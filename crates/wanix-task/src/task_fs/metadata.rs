use wanix_fs::{FsError, FsResult, Metadata, NormalizedPath};

use crate::TaskFs;
use crate::task_files::{
    TASK_FILE_READ_ONLY_MODE, directory_metadata, field_metadata, file_metadata,
};

enum MetadataTarget<'a> {
    RootDirectory,
    NewTaskFile(&'a str),
    TaskDirectory(&'a str),
    FdDirectory(&'a str),
    FdFile { selector: &'a str, fd: &'a str },
    Field { selector: &'a str, field: &'a str },
}

impl<'a> MetadataTarget<'a> {
    fn from_parts(parts: &'a [&'a str]) -> FsResult<Self> {
        match parts {
            [] | ["new"] => Ok(Self::RootDirectory),
            ["new", kind] => Ok(Self::NewTaskFile(kind)),
            [selector] => Ok(Self::TaskDirectory(selector)),
            [selector, "fd"] => Ok(Self::FdDirectory(selector)),
            [selector, "fd", fd] => Ok(Self::FdFile { selector, fd }),
            [selector, field] => Ok(Self::Field { selector, field }),
            _ => Err(FsError::NotFound),
        }
    }
}

impl TaskFs {
    pub(super) fn metadata_path(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        let parts = super::components(path);
        match MetadataTarget::from_parts(&parts)? {
            MetadataTarget::RootDirectory => Ok(directory_metadata()),
            MetadataTarget::NewTaskFile(kind) => self.new_task_metadata(kind),
            MetadataTarget::TaskDirectory(selector) | MetadataTarget::FdDirectory(selector) => {
                self.task_for_selector(selector)?;
                Ok(directory_metadata())
            }
            MetadataTarget::FdFile { selector, fd } => {
                let task = self.task_for_selector(selector)?;
                task.fd_metadata(super::parse_fd(fd)?)
            }
            MetadataTarget::Field { selector, field } => {
                let task = self.task_for_selector(selector)?;
                field_metadata(&task, field)
            }
        }
    }

    fn new_task_metadata(&self, kind: &str) -> FsResult<Metadata> {
        if self
            .table()
            .driver_kinds()
            .iter()
            .any(|driver| driver == kind)
        {
            Ok(file_metadata(0, TASK_FILE_READ_ONLY_MODE))
        } else {
            Err(FsError::NotFound)
        }
    }
}
