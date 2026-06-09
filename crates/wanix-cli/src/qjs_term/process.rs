use std::io::{Read, Write};
use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub(super) use unix::terminal_size_for_fd;
#[cfg(unix)]
pub(crate) use unix::{
    NonBlockingFd, ProcessStdinPoll, ProcessStdinRead, poll_process_stdin,
    read_process_stdin_after_poll,
};

use super::CliError;
#[cfg(unix)]
use super::pump::ProcessInputMode;
use super::pump::{
    TerminalPumpContext, TerminalPumpState, feed_terminal_batch_and_pump,
    feed_terminal_chunk_and_pump, task_exited,
};

pub(super) struct ProcessFeedContext<'a> {
    pub(super) process_stdin: &'a mut dyn Read,
    pub(super) terminal: &'a TermDevice,
    pub(super) terminal_id: &'a str,
    pub(super) runtime: &'a mut QuickJsTaskRuntime,
    pub(super) pump_state: &'a mut TerminalPumpState,
    pub(super) process_stdout: &'a mut dyn Write,
}

#[derive(Clone, Copy)]
enum ProcessFeedMode {
    RawBytes,
    Lines,
}

pub(super) fn run_process_raw_byte_feed_session_after_eval(
    context: ProcessFeedContext<'_>,
) -> Result<(), CliError> {
    run_process_feed_session_after_eval(context, ProcessFeedMode::RawBytes)
}

pub(super) fn run_process_line_feed_session_after_eval(
    context: ProcessFeedContext<'_>,
) -> Result<(), CliError> {
    run_process_feed_session_after_eval(context, ProcessFeedMode::Lines)
}

fn run_process_feed_session_after_eval(
    context: ProcessFeedContext<'_>,
    mode: ProcessFeedMode,
) -> Result<(), CliError> {
    let policy = context.pump_state.policy;
    #[cfg(unix)]
    if let ProcessInputMode::PollFd(input_fd) = policy.input_mode {
        return unix::run_process_polled_feed_session_after_eval(input_fd, context);
    }

    let mut pump_context = terminal_pump_context(
        context.terminal,
        context.terminal_id,
        context.runtime,
        context.process_stdout,
    );
    match mode {
        ProcessFeedMode::RawBytes => {
            run_process_raw_byte_feed(context.process_stdin, &mut pump_context, policy)
        }
        ProcessFeedMode::Lines => {
            run_process_line_feed(context.process_stdin, &mut pump_context, policy)
        }
    }
}

fn run_process_raw_byte_feed(
    process_stdin: &mut dyn Read,
    pump_context: &mut TerminalPumpContext<'_>,
    policy: super::pump::TerminalPumpPolicy,
) -> Result<(), CliError> {
    let mut byte = [0; 1];
    loop {
        let count = process_stdin.read(&mut byte).map_err(|error| {
            CliError::new(
                format!("failed to read process stdin raw bytes after eval: {error}"),
                1,
            )
        })?;
        if count == 0 {
            return Ok(());
        }
        feed_terminal_chunk_and_pump(pump_context, &byte[..count], policy)?;
        if task_exited(pump_context.runtime)? {
            break;
        }
    }
    Ok(())
}

pub(super) fn split_feed_lines(bytes: Vec<u8>) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            chunks.push(bytes[start..=index].to_vec());
            start = index + 1;
        }
    }
    if start < bytes.len() {
        chunks.push(bytes[start..].to_vec());
    }
    chunks
}

fn run_process_line_feed(
    process_stdin: &mut dyn Read,
    pump_context: &mut TerminalPumpContext<'_>,
    policy: super::pump::TerminalPumpPolicy,
) -> Result<(), CliError> {
    let mut line = Vec::new();
    while read_process_line_after_eval(process_stdin, &mut line)? {
        feed_terminal_batch_and_pump(pump_context, &[line.clone()], policy)?;
        if task_exited(pump_context.runtime)? {
            break;
        }
    }
    Ok(())
}

fn read_process_line_after_eval(
    process_stdin: &mut dyn Read,
    line: &mut Vec<u8>,
) -> Result<bool, CliError> {
    line.clear();
    let mut byte = [0; 1];
    loop {
        let count = process_stdin.read(&mut byte).map_err(|error| {
            CliError::new(
                format!("failed to read process stdin lines after eval: {error}"),
                1,
            )
        })?;
        if count == 0 {
            return Ok(!line.is_empty());
        }
        line.push(byte[0]);
        if byte[0] == b'\n' {
            return Ok(true);
        }
    }
}

fn terminal_pump_context<'a>(
    terminal: &'a TermDevice,
    terminal_id: &'a str,
    runtime: &'a mut QuickJsTaskRuntime,
    process_stdout: &'a mut dyn Write,
) -> TerminalPumpContext<'a> {
    TerminalPumpContext {
        terminal,
        terminal_id,
        runtime,
        process_stdout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_feed_lines_preserves_newline_terminated_chunks() {
        assert_eq!(
            split_feed_lines(b"first\nsecond\n".to_vec()),
            [b"first\n".to_vec(), b"second\n".to_vec()]
        );
    }

    #[test]
    fn split_feed_lines_keeps_trailing_fragment_without_newline() {
        assert_eq!(
            split_feed_lines(b"first\nsecond".to_vec()),
            [b"first\n".to_vec(), b"second".to_vec()]
        );
    }

    #[test]
    fn split_feed_lines_drops_empty_input_without_synthetic_line() {
        assert!(split_feed_lines(Vec::new()).is_empty());
    }
}
