use wanix_fs::{File, FsError, FsResult, NormalizedPath, OpenOptions};

use crate::TaskFs;
use crate::task_files::{ControlFile, FdProxyFile, Field, FieldFile, FileAccess, NewTaskFile};

#[derive(Debug, Clone, Copy)]
enum OpenTaskField {
    Control,
    Field(Field),
    FdDirectory,
}

impl OpenTaskField {
    fn from_name(field: &str) -> FsResult<Self> {
        match field {
            "ctl" => Ok(Self::Control),
            "fd" => Ok(Self::FdDirectory),
            name => Field::from_name(name).map(Self::Field),
        }
    }
}

impl TaskFs {
    pub(super) fn open_path(
        &self,
        path: &NormalizedPath,
        options: OpenOptions,
    ) -> FsResult<Box<dyn File>> {
        let parts = super::components(path);
        match parts.as_slice() {
            [] | ["new"] => Err(FsError::IsDirectory),
            [selector] => self.open_task_directory(selector),
            ["new", kind] => self.open_new_task_file(kind, options),
            [selector, field] => self.open_task_field(selector, field, options),
            [selector, "fd", fd] => self.open_task_fd(selector, fd, options),
            _ => Err(FsError::NotFound),
        }
    }

    fn open_task_directory(&self, selector: &str) -> FsResult<Box<dyn File>> {
        match self.task_for_selector(selector) {
            Ok(_) => Err(FsError::IsDirectory),
            Err(error) => Err(error),
        }
    }

    fn open_new_task_file(&self, kind: &str, options: OpenOptions) -> FsResult<Box<dyn File>> {
        require_read_only(options)?;
        if self
            .table()
            .driver_kinds()
            .iter()
            .any(|driver| driver == kind)
        {
            Ok(Box::new(NewTaskFile::new(
                self.table().clone(),
                self.current(),
                kind,
            )))
        } else {
            Err(FsError::NotFound)
        }
    }

    fn open_task_field(
        &self,
        selector: &str,
        field: &str,
        options: OpenOptions,
    ) -> FsResult<Box<dyn File>> {
        let task = self.task_for_selector(selector)?;
        match OpenTaskField::from_name(field)? {
            OpenTaskField::Control => Ok(Box::new(ControlFile::new(
                self.table().clone(),
                task,
                access(options)?,
            ))),
            OpenTaskField::Field(field) if field.is_read_only() => {
                require_read_only(options)?;
                Ok(Box::new(FieldFile::new(
                    task,
                    field,
                    FileAccess::read_only(),
                )))
            }
            OpenTaskField::Field(field) => {
                Ok(Box::new(FieldFile::new(task, field, access(options)?)))
            }
            OpenTaskField::FdDirectory => Err(FsError::IsDirectory),
        }
    }

    fn open_task_fd(
        &self,
        selector: &str,
        fd: &str,
        options: OpenOptions,
    ) -> FsResult<Box<dyn File>> {
        let task = self.task_for_selector(selector)?;
        let fd = super::parse_fd(fd)?;
        let file = task.fd_file(fd)?;
        Ok(Box::new(FdProxyFile::new(file, access(options)?)))
    }
}

fn access(options: OpenOptions) -> FsResult<FileAccess> {
    if options.create {
        return Err(FsError::AlreadyExists);
    }
    if options.truncate && !options.write {
        return Err(FsError::PermissionDenied);
    }
    Ok(FileAccess::new(options.read, options.write))
}

fn require_read_only(options: OpenOptions) -> FsResult<()> {
    let access = access(options)?;
    access.can_read()?;
    if options.write || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}
