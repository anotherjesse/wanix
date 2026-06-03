use wanix_fs::{File, FsResult, Metadata};
use wanix_task::{Fd, Task};
use wanix_wasi::WasiConfig;

use crate::task_wasi_argv;

pub(crate) fn task_wasi_config(task: &Task) -> WasiConfig {
    let mut config = WasiConfig::new(task.namespace())
        .with_args(task_wasi_argv(task))
        .with_env(task.env());
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

    fn metadata(&self) -> FsResult<Metadata> {
        self.task.fd_metadata(self.fd)
    }
}
