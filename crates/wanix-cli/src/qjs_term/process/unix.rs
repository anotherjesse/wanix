use std::io::{self, Read};
use std::time::Duration;

mod fd_mode;
mod stdin_poll;
mod terminal_size;

use fd_mode::NonBlockingFd;
use stdin_poll::{ProcessStdinPoll, poll_process_stdin};

use super::super::pump::{
    TerminalPumpContext, TerminalPumpState, feed_terminal_chunk_and_pump, pump_terminal_idle,
    pump_terminal_resize_if_changed, task_exited,
};
use super::super::{CliError, QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS};
use super::ProcessFeedContext;

pub(in crate::qjs_term) use terminal_size::terminal_size_for_fd;

const PROCESS_STDIN_READ_CHUNK_BYTES: usize = 1024;

pub(super) fn run_process_polled_feed_session_after_eval(
    input_fd: libc::c_int,
    context: ProcessFeedContext<'_>,
) -> Result<(), CliError> {
    let _nonblocking = NonBlockingFd::enter(input_fd)?;
    let mut bytes = [0; PROCESS_STDIN_READ_CHUNK_BYTES];
    let poll_timeout = Duration::from_millis(QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS);
    let policy = context.pump_state.policy;
    let idle_budget = qjs_shell_idle_event_loop_budget(policy.event_loop_wait_budget);
    let mut session = PolledFeedSession {
        process_stdin: context.process_stdin,
        pump_context: TerminalPumpContext {
            terminal: context.terminal,
            terminal_id: context.terminal_id,
            runtime: context.runtime,
            process_stdout: context.process_stdout,
        },
        pump_state: context.pump_state,
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
        if task_exited(session.pump_context.runtime)? {
            return Ok(());
        }
    }
}

struct PolledFeedSession<'a> {
    process_stdin: &'a mut dyn Read,
    pump_context: TerminalPumpContext<'a>,
    pump_state: &'a mut TerminalPumpState,
    policy: super::super::pump::TerminalPumpPolicy,
    idle_budget: Duration,
}

impl PolledFeedSession<'_> {
    fn feed_ready_stdin(&mut self, bytes: &mut [u8]) -> Result<bool, CliError> {
        self.pump_resize()?;
        match read_process_stdin_after_poll(self.process_stdin, bytes)? {
            ProcessStdinRead::Bytes(count) => {
                feed_terminal_chunk_and_pump(&mut self.pump_context, &bytes[..count], self.policy)?;
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
        let policy = super::super::pump::TerminalPumpPolicy {
            event_loop_wait_budget: self.idle_budget,
            ..self.policy
        };
        pump_terminal_idle(&mut self.pump_context, policy)
    }

    fn pump_resize(&mut self) -> Result<(), CliError> {
        pump_terminal_resize_if_changed(&mut self.pump_context, self.pump_state)
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

fn qjs_shell_idle_event_loop_budget(configured: Duration) -> Duration {
    if configured.is_zero() {
        Duration::from_millis(QJS_SHELL_IDLE_EVENT_LOOP_BUDGET_MS)
    } else {
        configured
    }
}
