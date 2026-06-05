use std::io;

use super::super::super::CliError;

#[derive(Debug)]
pub(super) struct NonBlockingFd {
    fd: libc::c_int,
    original_flags: libc::c_int,
}

impl NonBlockingFd {
    pub(super) fn enter(fd: libc::c_int) -> Result<Self, CliError> {
        let original_flags = process_stdin_flags(fd)?;
        set_process_stdin_flags(
            fd,
            original_flags | libc::O_NONBLOCK,
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

fn process_stdin_flags(fd: libc::c_int) -> Result<libc::c_int, CliError> {
    fcntl_status_flags(fd, libc::F_GETFL, 0, "read process stdin flags")
}

fn set_process_stdin_flags(
    fd: libc::c_int,
    flags: libc::c_int,
    action: &str,
) -> Result<(), CliError> {
    fcntl_status_flags(fd, libc::F_SETFL, flags, action).map(|_| ())
}

fn fcntl_status_flags(
    fd: libc::c_int,
    command: libc::c_int,
    flags: libc::c_int,
    action: &str,
) -> Result<libc::c_int, CliError> {
    // SAFETY: `fcntl` observes or updates status flags for the supplied fd.
    let result = unsafe { libc::fcntl(fd, command, flags) };
    if result < 0 {
        return Err(CliError::new(
            format!("failed to {action}: {}", io::Error::last_os_error()),
            1,
        ));
    }
    Ok(result)
}
