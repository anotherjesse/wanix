use std::time::Duration;

use super::super::super::CliError;
use crate::unix_fd::with_borrowed_fd;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProcessStdinPoll {
    Ready,
    Idle,
}

const MILLIS_PER_SECOND: i64 = 1_000;
const NANOS_PER_MILLI: i64 = 1_000_000;

pub(super) fn poll_process_stdin(
    input_fd: libc::c_int,
    timeout: Duration,
) -> Result<ProcessStdinPoll, CliError> {
    loop {
        let polled = with_borrowed_fd(input_fd, |fd| {
            let mut poll_fd = PollFd::from_borrowed_fd(fd, PollFlags::IN);
            let ready = poll(
                std::slice::from_mut(&mut poll_fd),
                Some(&poll_timeout(timeout)),
            )?;
            Ok::<_, Errno>((ready, poll_fd.revents()))
        });
        let (ready, revents) = match polled {
            Ok(polled) => polled,
            Err(error) if error == Errno::INTR => continue,
            Err(error) => {
                return Err(CliError::new(
                    format!("failed to poll process stdin: {error}"),
                    1,
                ));
            }
        };
        if ready == 0 {
            return Ok(ProcessStdinPoll::Idle);
        }
        if revents.contains(PollFlags::NVAL) {
            return Err(CliError::new("failed to poll process stdin: invalid fd", 1));
        }
        if revents.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
            return Ok(ProcessStdinPoll::Ready);
        }
        return Ok(ProcessStdinPoll::Idle);
    }
}

fn poll_timeout(timeout: Duration) -> Timespec {
    let millis = timeout.as_millis();
    let millis = millis.min(libc::c_int::MAX as u128) as i64;
    Timespec {
        tv_sec: millis / MILLIS_PER_SECOND,
        tv_nsec: (millis % MILLIS_PER_SECOND) * NANOS_PER_MILLI,
    }
}
