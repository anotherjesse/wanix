use wanix_fs::{File, FsResult, Metadata};

use super::{TASK_FILE_READ_ONLY_MODE, file_metadata, read_from_slice};
use crate::Task;

/// `#task/<id>/wait`: a read blocks until the task records an exit status,
/// then serves that status (`<code>\n`) to end-of-file.
///
/// This is the wait half of a detached start (`ctl` `start &`): the launcher
/// returns immediately and any observer parks here until the task is done.
/// Unlike `exit` — a non-blocking snapshot kept readable for inspection (`ps`,
/// the cockpit) — `wait` is the synchronization point, so waiting on a task
/// that never starts blocks until something records its exit.
#[derive(Debug)]
pub(crate) struct WaitFile {
    task: Task,
    data: Option<Vec<u8>>,
    offset: usize,
}

impl WaitFile {
    pub(crate) fn new(task: Task) -> Self {
        Self {
            task,
            data: None,
            offset: 0,
        }
    }
}

impl File for WaitFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.data.is_none() {
            let exit = self.task.wait_exit()?;
            self.data = Some(format!("{exit}\n").into_bytes());
        }
        let Some(data) = self.data.as_ref() else {
            return Ok(0);
        };
        read_from_slice(data, &mut self.offset, buf)
    }

    fn read_ready(&self) -> FsResult<bool> {
        Ok(self.data.is_some() || !self.task.exit().is_empty())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, TASK_FILE_READ_ONLY_MODE))
    }
}
