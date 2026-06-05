//! Builds a [`WasiConfig`] from a Wanix [`Task`], shared by every WASI task
//! runtime (QuickJS and the compiled-wasm driver).
//!
//! The task namespace backs WASI, the task cwd is the root preopen source,
//! argv/env come from the shared [`wanix_task`] command extraction, and task
//! fds 0/1/2 are wired as guest stdio. Dynamically opened guest fds are mirrored
//! into the task fd table through a [`WasiFdObserver`] so they are observable at
//! the task level and released on guest close. Directory fds stay WASI-internal:
//! only regular-file opens reach the observer, matching ADR 0002's fd-mirroring
//! contract for both runtimes.

use wanix_fs::{File, FsError, FsResult, Metadata, NormalizedPath};
use wanix_task::{Fd, Task, task_wasi_argv, task_wasi_cwd, task_wasi_env};

use crate::{Errno, WasiConfig, WasiFd, WasiFdObserver, WasiFile};

/// Builds the live WASI config for a task: namespace, cwd preopen, argv, env,
/// stdio fds, and dynamic fd mirroring.
#[must_use]
pub fn task_wasi_config(task: &Task) -> WasiConfig {
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

/// Mirrors dynamic WASI regular-file fds into the owning task's fd table.
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

/// Backs a guest stdio fd with the live task fd-table entry it mirrors.
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs, NormalizedPath, OpenOptions};
    use wanix_task::{Fd, TaskTable};
    use wanix_vfs::BindOptions;

    use super::task_wasi_config;
    use crate::{WasiCtx, WasiFd, WasiOpenOptions};

    fn task_with_data() -> wanix_task::Task {
        let table = TaskTable::new();
        table.register_noop_driver("wasm").unwrap();
        let task = table.allocate_root("wasm").unwrap();
        let root = Arc::new(MemFs::new());
        root.write_file("data.txt", b"from mirrored fd").unwrap();
        task.bind(root, ".", ".", BindOptions::default()).unwrap();
        task
    }

    #[test]
    fn dynamic_file_fd_is_mirrored_into_task_table_and_released_on_close() {
        let task = task_with_data();
        let mut ctx = WasiCtx::new(task_wasi_config(&task));

        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();

        // The guest-opened fd is now observable at the task level.
        assert_eq!(fd, WasiFd::new(4));
        assert_eq!(task.fd_numbers(), [Fd::new(4)]);
        let mut buf = [0; 32];
        let count = task.read_fd(Fd::new(4), &mut buf).unwrap();
        assert_eq!(&buf[..count], b"from mirrored fd");

        // Closing the guest fd releases the mirrored task fd.
        ctx.fd_close(fd).unwrap();
        assert!(task.fd_numbers().is_empty());
    }

    #[test]
    fn mirroring_skips_an_already_occupied_task_fd_number() {
        let task = task_with_data();
        let existing = Arc::new(MemFs::new());
        existing.write_file("existing.txt", b"existing fd").unwrap();
        task.insert_fd(
            Fd::new(4),
            existing
                .open(
                    &NormalizedPath::new("existing.txt").unwrap(),
                    OpenOptions::read(),
                )
                .unwrap(),
            NormalizedPath::new("existing.txt").unwrap(),
        )
        .unwrap();

        let mut ctx = WasiCtx::new(task_wasi_config(&task));
        let fd = ctx
            .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
            .unwrap();

        // The dynamic open lands at the next free number, leaving fd 4 intact.
        assert_eq!(fd, WasiFd::new(5));
        assert_eq!(task.fd_numbers(), [Fd::new(4), Fd::new(5)]);
        assert_eq!(task.fd_path(Fd::new(4)).unwrap().as_str(), "existing.txt");
        assert_eq!(task.fd_path(Fd::new(5)).unwrap().as_str(), "data.txt");

        ctx.fd_close(fd).unwrap();
        assert_eq!(task.fd_numbers(), [Fd::new(4)]);
    }

    #[test]
    fn dropping_the_ctx_releases_mirrored_dynamic_fds() {
        let task = task_with_data();
        {
            let mut ctx = WasiCtx::new(task_wasi_config(&task));
            let fd = ctx
                .path_open(WasiFd::ROOT, "data.txt", WasiOpenOptions::read())
                .unwrap();
            assert_eq!(fd, WasiFd::new(4));
            assert_eq!(task.fd_numbers(), [Fd::new(4)]);
        }
        assert!(task.fd_numbers().is_empty());
    }
}
