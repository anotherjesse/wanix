use super::super::super::CliError;
use crate::unix_fd::with_borrowed_fd;
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};

#[derive(Debug)]
pub(crate) struct NonBlockingFd {
    fd: libc::c_int,
    original_flags: OFlags,
}

impl NonBlockingFd {
    pub(crate) fn enter(fd: libc::c_int) -> Result<Self, CliError> {
        let original_flags = process_stdin_flags(fd)?;
        set_process_stdin_flags(
            fd,
            original_flags | OFlags::NONBLOCK,
            "enter nonblocking process stdin mode",
        )?;
        Ok(Self { fd, original_flags })
    }
}

impl Drop for NonBlockingFd {
    fn drop(&mut self) {
        let _ =
            set_process_stdin_flags(self.fd, self.original_flags, "restore process stdin flags");
    }
}

fn process_stdin_flags(fd: libc::c_int) -> Result<OFlags, CliError> {
    with_borrowed_fd(fd, |fd| fcntl_getfl(fd))
        .map_err(|error| CliError::new(format!("failed to read process stdin flags: {error}"), 1))
}

fn set_process_stdin_flags(fd: libc::c_int, flags: OFlags, action: &str) -> Result<(), CliError> {
    with_borrowed_fd(fd, |fd| fcntl_setfl(fd, flags))
        .map_err(|error| CliError::new(format!("failed to {action}: {error}"), 1))
}
