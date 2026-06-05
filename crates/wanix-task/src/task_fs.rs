use std::fmt;

use wanix_fs::{
    DirEntry, File, FileSystem, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

use crate::task_files::{
    TASK_FD_FILE_MODE, TASK_FILE_READ_ONLY_MODE, directory_metadata, field_metadata, file_metadata,
    task_entries,
};
use crate::{Task, TaskId, TaskTable};

mod open;

/// Filesystem view for the Wanix `#task` service.
#[derive(Clone)]
pub struct TaskFs {
    table: TaskTable,
    current: Option<TaskId>,
}

impl fmt::Debug for TaskFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskFs")
            .field("current", &self.current)
            .finish()
    }
}

impl TaskFs {
    /// Creates a task filesystem view.
    #[must_use]
    pub fn new(table: TaskTable, current: Option<TaskId>) -> Self {
        Self { table, current }
    }

    /// Returns the current task id for this filesystem view.
    #[must_use]
    pub fn current(&self) -> Option<TaskId> {
        self.current
    }

    /// Returns the shared task table.
    #[must_use]
    pub fn table(&self) -> &TaskTable {
        &self.table
    }
}

impl FileSystem for TaskFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        self.open_path(path, options)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        let parts = components(path);
        match parts.as_slice() {
            [] | ["new"] => Ok(directory_metadata()),
            ["new", kind]
                if self
                    .table
                    .driver_kinds()
                    .iter()
                    .any(|driver| driver == kind) =>
            {
                Ok(file_metadata(0, TASK_FILE_READ_ONLY_MODE))
            }
            [selector] => {
                self.task_for_selector(selector)?;
                Ok(directory_metadata())
            }
            [selector, "fd"] => {
                self.task_for_selector(selector)?;
                Ok(directory_metadata())
            }
            [selector, "fd", fd] => {
                let task = self.task_for_selector(selector)?;
                task.fd_metadata(parse_fd(fd)?)
            }
            [selector, field] => {
                let task = self.task_for_selector(selector)?;
                field_metadata(&task, field)
            }
            _ => Err(FsError::NotFound),
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        let parts = components(path);
        match parts.as_slice() {
            [] => Ok(self.root_entries()),
            ["new"] => Ok(self
                .table
                .driver_kinds()
                .into_iter()
                .map(|kind| DirEntry::new(kind, file_metadata(0, TASK_FILE_READ_ONLY_MODE)))
                .collect()),
            [selector] => {
                self.task_for_selector(selector)?;
                Ok(task_entries())
            }
            [selector, "fd"] => {
                let task = self.task_for_selector(selector)?;
                Ok(task
                    .fd_numbers()
                    .into_iter()
                    .map(|fd| {
                        DirEntry::new(fd.get().to_string(), file_metadata(0, TASK_FD_FILE_MODE))
                    })
                    .collect())
            }
            _ => Err(FsError::NotFound),
        }
    }
}

impl TaskFs {
    fn root_entries(&self) -> Vec<DirEntry> {
        let mut entries = vec![DirEntry::new("new", directory_metadata())];
        entries.extend(
            self.table
                .tasks()
                .into_iter()
                .map(|task| DirEntry::new(task.id().get().to_string(), directory_metadata())),
        );
        if self.current.is_some() {
            entries.push(DirEntry::new("self", directory_metadata()));
        }
        entries.sort_by(|a, b| a.name().cmp(b.name()));
        entries
    }

    fn task_for_selector(&self, selector: &str) -> FsResult<Task> {
        let id = if selector == "self" {
            self.current.ok_or(FsError::NotFound)?
        } else {
            parse_task_id(selector)?
        };
        self.table.get(id).ok_or(FsError::NotFound)
    }
}

fn components(path: &NormalizedPath) -> Vec<&str> {
    if path.as_str() == "." {
        Vec::new()
    } else {
        path.as_str().split('/').collect()
    }
}

fn parse_fd(fd: &str) -> FsResult<crate::Fd> {
    Ok(crate::Fd::new(fd.parse().map_err(|_| FsError::InvalidFd)?))
}

fn parse_task_id(task_id: &str) -> FsResult<TaskId> {
    let task_id = task_id.parse().map_err(|_| FsError::NotFound)?;
    if task_id == 0 {
        return Err(FsError::NotFound);
    }
    Ok(TaskId::new(task_id))
}
