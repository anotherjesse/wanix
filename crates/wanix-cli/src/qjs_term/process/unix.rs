use std::io::{self, Read, Write};
use std::time::Duration;

use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

use super::super::pump::{
    TermResize, TerminalPumpState, feed_terminal_chunk_and_pump, pump_terminal_idle,
    pump_terminal_resize_if_changed, task_exited,
};
use super::super::{CliError, QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS};

const PROCESS_STDIN_READ_CHUNK_BYTES: usize = 1024;

pub(super) fn run_process_polled_feed_session_after_eval(
    process_stdin: &mut dyn Read,
    input_fd: libc::c_int,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let _nonblocking = NonBlockingFd::enter(input_fd)?;
    let mut bytes = [0; PROCESS_STDIN_READ_CHUNK_BYTES];
    let poll_timeout = Duration::from_millis(QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS);
    let policy = pump_state.policy;
    let idle_budget = qjs_shell_idle_event_loop_budget(policy.event_loop_wait_budget);
    let mut session = PolledFeedSession {
        process_stdin,
        terminal,
        terminal_id,
        runtime,
        pump_state,
        process_stdout,
        policy,
        idle_budget,
    };
    session.pump_idle()?;
    loop {
        match poll_process_stdin(input_fd, poll_timeout)? {
            ProcessStdinPoll::Ready => {
                if session.feed_ready_stdin(&mut bytes)? {
                    return Ok(());
                }
            }
            ProcessStdinPoll::Idle => session.pump_idle()?,
        }
        if task_exited(session.runtime)? {
            return Ok(());
        }
    }
}

struct PolledFeedSession<'a> {
    process_stdin: &'a mut dyn Read,
    terminal: &'a TermDevice,
    terminal_id: &'a str,
    runtime: &'a mut QuickJsTaskRuntime,
    pump_state: &'a mut TerminalPumpState,
    process_stdout: &'a mut dyn Write,
    policy: super::super::pump::TerminalPumpPolicy,
    idle_budget: Duration,
}

impl PolledFeedSession<'_> {
    fn feed_ready_stdin(&mut self, bytes: &mut [u8]) -> Result<bool, CliError> {
        self.pump_resize()?;
        match read_process_stdin_after_poll(self.process_stdin, bytes)? {
            ProcessStdinRead::Bytes(count) => {
                feed_terminal_chunk_and_pump(
                    self.terminal,
                    self.terminal_id,
                    self.runtime,
                    &bytes[..count],
                    self.policy.ready_io_turns,
                    self.policy.event_loop_wait_budget,
                    self.process_stdout,
                )?;
                Ok(false)
            }
            ProcessStdinRead::Eof => Ok(true),
            ProcessStdinRead::Interrupted => Ok(false),
            ProcessStdinRead::Idle => {
                self.pump_idle()?;
                Ok(false)
            }
        }
    }

    fn pump_idle(&mut self) -> Result<(), CliError> {
        self.pump_resize()?;
        pump_terminal_idle(
            self.terminal,
            self.terminal_id,
            self.runtime,
            self.policy.ready_io_turns,
            self.idle_budget,
            self.process_stdout,
        )
    }

    fn pump_resize(&mut self) -> Result<(), CliError> {
        pump_terminal_resize_if_changed(
            self.terminal,
            self.terminal_id,
            self.runtime,
            self.pump_state,
            self.process_stdout,
        )
    }
}

enum ProcessStdinRead {
    Bytes(usize),
    Eof,
    Interrupted,
    Idle,
}

fn read_process_stdin_after_poll(
    process_stdin: &mut dyn Read,
    bytes: &mut [u8],
) -> Result<ProcessStdinRead, CliError> {
    match process_stdin.read(bytes) {
        Ok(0) => Ok(ProcessStdinRead::Eof),
        Ok(count) => Ok(ProcessStdinRead::Bytes(count)),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
            Ok(ProcessStdinRead::Interrupted)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(ProcessStdinRead::Idle),
        Err(error) => Err(CliError::new(
            format!("failed to read process stdin after eval: {error}"),
            1,
        )),
    }
}

#[derive(Debug)]
struct NonBlockingFd {
    fd: libc::c_int,
    original_flags: libc::c_int,
}

impl NonBlockingFd {
    fn enter(fd: libc::c_int) -> Result<Self, CliError> {
        let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if original_flags < 0 {
            return Err(CliError::new(
                format!(
                    "failed to read process stdin flags: {}",
                    io::Error::last_os_error()
                ),
                1,
            ));
        }
        let nonblocking_flags = original_flags | libc::O_NONBLOCK;
        if unsafe { libc::fcntl(fd, libc::F_SETFL, nonblocking_flags) } < 0 {
            return Err(CliError::new(
                format!(
                    "failed to enter nonblocking process stdin mode: {}",
                    io::Error::last_os_error()
                ),
                1,
            ));
        }
        Ok(Self { fd, original_flags })
    }
}

impl Drop for NonBlockingFd {
    fn drop(&mut self) {
        let _ = unsafe { libc::fcntl(self.fd, libc::F_SETFL, self.original_flags) };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessStdinPoll {
    Ready,
    Idle,
}

fn poll_process_stdin(
    input_fd: libc::c_int,
    timeout: Duration,
) -> Result<ProcessStdinPoll, CliError> {
    let mut poll_fd = libc::pollfd {
        fd: input_fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
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

fn qjs_shell_idle_event_loop_budget(configured: Duration) -> Duration {
    if configured.is_zero() {
        Duration::from_millis(QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS)
    } else {
        configured
    }
}

pub(in crate::qjs_term) fn terminal_size_for_fd(
    fd: libc::c_int,
) -> Result<Option<TermResize>, CliError> {
    // SAFETY: `isatty` only observes the supplied file descriptor.
    if unsafe { libc::isatty(fd) } == 0 {
        return Ok(None);
    }
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    // SAFETY: `size` points to valid writable memory for TIOCGWINSZ.
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
        return Err(CliError::new(
            format!(
                "failed to read native terminal size: {}",
                io::Error::last_os_error()
            ),
            1,
        ));
    }
    // SAFETY: ioctl succeeded and initialized the winsize value.
    let size = unsafe { size.assume_init() };
    if size.ws_col == 0 || size.ws_row == 0 {
        return Ok(None);
    }
    Ok(Some(TermResize {
        columns: size.ws_col,
        rows: size.ws_row,
    }))
}
