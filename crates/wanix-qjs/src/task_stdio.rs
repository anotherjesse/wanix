use wanix_fs::{File, FsError, FsResult, Metadata, NormalizedPath};
use wanix_task::{Fd, Task};
use wanix_wasi::{Errno, WasiConfig, WasiFd, WasiFdObserver, WasiFile};

use crate::{task_wasi_argv, task_wasi_cwd, task_wasi_env};

pub(crate) fn task_wasi_config(task: &Task) -> WasiConfig {
    let mut config = WasiConfig::new(task.namespace())
        .with_root_preopen_source(task_wasi_cwd(task))
        .with_args(task_wasi_argv(task))
        .with_env(task_wasi_env(task))
        .with_fd_observer(TaskWasiFdMirror::new(task.clone()));
    let fds = task.fd_numbers();
    if fds.contains(&Fd::STDIN) {
        config = config.with_stdin(
            Box::new(TaskFdFile::new(task.clone(), Fd::STDIN)),
            "task fd 0",
        );
    }
    if fds.contains(&Fd::STDOUT) {
        config = config.with_stdout(
            Box::new(TaskFdFile::new(task.clone(), Fd::STDOUT)),
            "task fd 1",
        );
    }
    if fds.contains(&Fd::STDERR) {
        config = config.with_stderr(
            Box::new(TaskFdFile::new(task.clone(), Fd::STDERR)),
            "task fd 2",
        );
    }
    config
}

#[derive(Debug, Clone)]
struct TaskWasiFdMirror {
    task: Task,
}

impl TaskWasiFdMirror {
    fn new(task: Task) -> Self {
        Self { task }
    }
}

impl WasiFdObserver for TaskWasiFdMirror {
    fn file_fd_available(&self, fd: WasiFd) -> bool {
        !self.task.fd_numbers().contains(&Fd::new(fd.get()))
    }

    fn file_opened(&self, fd: WasiFd, file: WasiFile, path: &NormalizedPath) -> Result<(), Errno> {
        let task_fd = Fd::new(fd.get());
        self.task
            .insert_fd_if_vacant(task_fd, Box::new(file), path.clone())
            .map_err(Errno::from)
    }

    fn fd_closed(&self, fd: WasiFd) -> Result<(), Errno> {
        match self.task.close_fd(Fd::new(fd.get())) {
            Ok(()) | Err(FsError::InvalidFd) => Ok(()),
            Err(err) => Err(Errno::from(err)),
        }
    }
}

#[derive(Debug)]
struct TaskFdFile {
    task: Task,
    fd: Fd,
}

impl TaskFdFile {
    fn new(task: Task, fd: Fd) -> Self {
        Self { task, fd }
    }
}

impl File for TaskFdFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.task.read_fd(self.fd, buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.task.write_fd(self.fd, buf)
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.task.fd_read_ready(self.fd)
    }

    fn write_ready(&self) -> FsResult<bool> {
        self.task.fd_write_ready(self.fd)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        self.task.fd_metadata(self.fd)
    }
}
