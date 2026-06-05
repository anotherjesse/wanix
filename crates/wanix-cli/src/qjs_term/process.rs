use std::io::{Read, Write};
use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub(super) use unix::terminal_size_for_fd;

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

pub(super) fn run_process_raw_byte_feed_session_after_eval(
    context: ProcessFeedContext<'_>,
) -> Result<(), CliError> {
    let policy = context.pump_state.policy;
    #[cfg(unix)]
    if let ProcessInputMode::PollFd(input_fd) = policy.input_mode {
        return unix::run_process_polled_feed_session_after_eval(input_fd, context);
    }

    let mut pump_context = TerminalPumpContext {
        terminal: context.terminal,
        terminal_id: context.terminal_id,
        runtime: context.runtime,
        process_stdout: context.process_stdout,
    };
    let mut byte = [0; 1];
    loop {
        let count = context.process_stdin.read(&mut byte).map_err(|error| {
            CliError::new(
                format!("failed to read process stdin raw bytes after eval: {error}"),
                1,
            )
        })?;
        if count == 0 {
            return Ok(());
        }
        feed_terminal_chunk_and_pump(&mut pump_context, &byte[..count], policy)?;
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

pub(super) fn run_process_line_feed_session_after_eval(
    context: ProcessFeedContext<'_>,
) -> Result<(), CliError> {
    let policy = context.pump_state.policy;
    #[cfg(unix)]
    if let ProcessInputMode::PollFd(input_fd) = policy.input_mode {
        return unix::run_process_polled_feed_session_after_eval(input_fd, context);
    }

    let mut pump_context = TerminalPumpContext {
        terminal: context.terminal,
        terminal_id: context.terminal_id,
        runtime: context.runtime,
        process_stdout: context.process_stdout,
    };
    let mut line = Vec::new();
    while read_process_line_after_eval(context.process_stdin, &mut line)? {
        feed_terminal_batch_and_pump(&mut pump_context, &[line.clone()], policy)?;
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
