//! Capturing a CPU job's stdio into in-memory buffers the acceptor can drain.
//!
//! The current task model writes a guest's stdout/stderr to files in its fd
//! table during [`wanix_task::TaskDriver::start`], then returns. So the acceptor
//! captures output exactly as the local CLI does: it inserts [`MemFs`]-backed
//! files at fds 1 and 2 before starting, and after `start` returns it reads those
//! buffers and ships them as [`crate::CpuEvent`] frames. This is the batch model
//! the blueprint requires v1 to be honest about — output is delivered after the
//! job completes, not incrementally.

use std::sync::Arc;

use wanix_fs::{FileSystem, FsResult, MemFs, NormalizedPath, OpenOptions};
use wanix_task::{Fd, Task};

/// In-memory capture buffers for a job's standard output and error.
pub(crate) struct CapturedStdio {
    stdout: Arc<MemFs>,
    stderr: Arc<MemFs>,
}

impl CapturedStdio {
    /// Inserts fresh stdout/stderr capture files into `task`'s fd table.
    ///
    /// Mirrors the CLI's `attach_task_stdio`: each stream is a one-file `MemFs`
    /// opened read-write and installed at the conventional fd, so the task
    /// driver's WASI config picks it up as fd 1 / fd 2.
    pub(crate) fn attach(task: &Task) -> FsResult<Self> {
        let stdout = attach_stream(task, Fd::STDOUT, "stdout")?;
        let stderr = attach_stream(task, Fd::STDERR, "stderr")?;
        Ok(Self { stdout, stderr })
    }

    /// Reads the captured standard-output bytes after the job has run.
    pub(crate) fn stdout_bytes(&self) -> FsResult<Vec<u8>> {
        read_capture(&self.stdout, "stdout")
    }

    /// Reads the captured standard-error bytes after the job has run.
    pub(crate) fn stderr_bytes(&self) -> FsResult<Vec<u8>> {
        read_capture(&self.stderr, "stderr")
    }
}

/// Builds a one-file `MemFs`, installs it at `fd`, and returns the buffer handle.
fn attach_stream(task: &Task, fd: Fd, name: &str) -> FsResult<Arc<MemFs>> {
    let fs = Arc::new(MemFs::new());
    fs.write_file(name, b"")?;
    let path = NormalizedPath::new(name)?;
    let file = fs.open(&path, OpenOptions::read_write())?;
    task.insert_fd(fd, file, path)?;
    Ok(fs)
}

/// Reads the whole capture file back out of its `MemFs`.
fn read_capture(fs: &Arc<MemFs>, name: &str) -> FsResult<Vec<u8>> {
    fs.read_file(name)
}

#[cfg(test)]
mod tests {
    use wanix_task::TaskTable;

    use super::*;

    #[test]
    fn captures_what_a_task_writes_to_its_fds() {
        let table = TaskTable::new();
        table.register_noop_driver("noop").unwrap();
        let task = table.allocate_root("noop").unwrap();
        let captured = CapturedStdio::attach(&task).unwrap();
        // Simulate a driver writing to stdout/stderr.
        task.write_fd(Fd::STDOUT, b"out").unwrap();
        task.write_fd(Fd::STDERR, b"err").unwrap();
        assert_eq!(captured.stdout_bytes().unwrap(), b"out");
        assert_eq!(captured.stderr_bytes().unwrap(), b"err");
    }
}
