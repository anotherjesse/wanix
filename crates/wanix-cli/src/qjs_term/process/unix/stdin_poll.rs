use std::io;
use std::time::Duration;

use super::super::super::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProcessStdinPoll {
    Ready,
    Idle,
}

pub(super) fn poll_process_stdin(
    input_fd: libc::c_int,
    timeout: Duration,
) -> Result<ProcessStdinPoll, CliError> {
    let mut poll_fd = libc::pollfd {
        fd: input_fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: `poll_fd` points to one valid pollfd entry for the duration of the call.
        let result = unsafe { libc::poll(&mut poll_fd, 1, poll_timeout_millis(timeout)) };
        if result == 0 {
            return Ok(ProcessStdinPoll::Idle);
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(CliError::new(
                format!("failed to poll process stdin: {error}"),
                1,
            ));
        }
        if poll_fd.revents & libc::POLLNVAL != 0 {
            return Err(CliError::new("failed to poll process stdin: invalid fd", 1));
        }
        if poll_fd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            return Ok(ProcessStdinPoll::Ready);
        }
        return Ok(ProcessStdinPoll::Idle);
    }
}

fn poll_timeout_millis(timeout: Duration) -> libc::c_int {
    let millis = timeout.as_millis();
    millis.min(libc::c_int::MAX as u128) as libc::c_int
}
