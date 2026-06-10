use wanix_fs::{File, FileSystem, FsResult, Metadata, NormalizedPath, OpenOptions};

use crate::{Fd, OpenFile};

use super::Task;

impl Task {
    /// Stores an open file in this task's fd table.
    pub fn open_fd(&self, file: Box<dyn File>, path: NormalizedPath) -> FsResult<Fd> {
        self.write_state(|state| Ok(state.fds.open(file, path)))
    }

    /// Installs a specific fd in this task's fd table.
    pub fn insert_fd(&self, fd: Fd, file: Box<dyn File>, path: NormalizedPath) -> FsResult<()> {
        self.write_state(|state| {
            state.fds.insert_at(fd, file, path);
            Ok(())
        })
    }

    /// Installs a specific fd when no file is currently present at that number.
    pub fn insert_fd_if_vacant(
        &self,
        fd: Fd,
        file: Box<dyn File>,
        path: NormalizedPath,
    ) -> FsResult<()> {
        self.write_state(|state| state.fds.insert_at_if_vacant(fd, file, path))
    }

    /// Opens a namespace path and installs it at a specific fd.
    ///
    /// This is the Rust task-service counterpart of wiring stdio through
    /// `ctl bind <src> fd/<n>`. The file is opened outside the task lock so
    /// service files can safely reenter task state.
    pub fn bind_fd_from_namespace(&self, source: impl AsRef<str>, fd: Fd) -> FsResult<()> {
        self.bind_fd_from_namespace_with(source, fd, fd_bind_open_options(fd))
    }

    /// Opens a namespace path with explicit options and installs it at a fd.
    ///
    /// Like [`bind_fd_from_namespace`](Self::bind_fd_from_namespace) but lets the
    /// caller choose the open mode instead of defaulting it from the fd number.
    /// This is needed to bind a strictly unidirectional source — for example a
    /// `#pipe` write end, which rejects a read-write open — onto fd 1/2.
    pub fn bind_fd_from_namespace_with(
        &self,
        source: impl AsRef<str>,
        fd: Fd,
        options: OpenOptions,
    ) -> FsResult<()> {
        let source = NormalizedPath::new(source)?;
        let namespace = self.namespace();
        let file = namespace.open(&source, options)?;
        self.insert_fd(fd, file, source)
    }

    /// Closes an fd.
    pub fn close_fd(&self, fd: Fd) -> FsResult<()> {
        self.write_state(|state| state.fds.close(fd))
    }

    /// Reads from an fd.
    pub fn read_fd(&self, fd: Fd, buf: &mut [u8]) -> FsResult<usize> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.read(buf)
    }

    /// Writes to an fd.
    pub fn write_fd(&self, fd: Fd, buf: &[u8]) -> FsResult<usize> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.write(buf)
    }

    /// Returns whether reading from an fd would produce data now.
    pub fn fd_read_ready(&self, fd: Fd) -> FsResult<bool> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.read_ready()
    }

    /// Returns whether writing to an fd can be attempted now.
    pub fn fd_write_ready(&self, fd: Fd) -> FsResult<bool> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.write_ready()
    }

    /// Returns metadata for an open fd.
    pub fn fd_metadata(&self, fd: Fd) -> FsResult<Metadata> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        file.metadata()
    }

    /// Returns a cloneable handle for an open fd.
    ///
    /// The returned handle remains valid if the fd number is later closed or
    /// replaced in this task. Clones share the same underlying file and offset.
    pub fn fd_file(&self, fd: Fd) -> FsResult<OpenFile> {
        self.read_state(|state| state.fds.file(fd))?
    }

    /// Returns the path associated with an open fd.
    pub fn fd_path(&self, fd: Fd) -> FsResult<NormalizedPath> {
        let file = self.read_state(|state| state.fds.file(fd))??;
        Ok(file.path().clone())
    }

    /// Returns sorted open fd numbers.
    #[must_use]
    pub fn fd_numbers(&self) -> Vec<Fd> {
        self.read_state(|state| state.fds.fds())
            .expect("task state lock should be readable")
    }

    /// Reports a host-level run failure on the task's stderr fd, best effort.
    ///
    /// A detached task (`TaskTable::start_detached`, the shell's `start &`)
    /// has no caller to surface a driver `Err` to — the table reduces it to an
    /// exit code — so without this the honest error (an over-cap verb refused
    /// at open, a module that fails to compile, a missing program) never
    /// reaches the operator. The Unix shape is an exec that fails after fork:
    /// the diagnostic goes to the child's inherited stderr. Drivers call this
    /// in their failure arm BEFORE `close_all_fds`. Write failures are
    /// swallowed: a task without fd 2 stays silent rather than failing the
    /// failure path.
    pub fn report_run_failure(&self, detail: &dyn std::fmt::Display) {
        let cmd = self.cmd();
        let program = cmd.split_whitespace().next().unwrap_or("task");
        let line = format!("wanix: {program}: {detail}\n");
        let mut bytes = line.as_bytes();
        while !bytes.is_empty() {
            match self.write_fd(Fd::STDERR, bytes) {
                Ok(0) | Err(_) => return,
                Ok(written) => bytes = &bytes[written..],
            }
        }
    }
}

fn fd_bind_open_options(fd: Fd) -> OpenOptions {
    if fd == Fd::STDIN {
        OpenOptions::read()
    } else {
        OpenOptions::read_write()
    }
}
