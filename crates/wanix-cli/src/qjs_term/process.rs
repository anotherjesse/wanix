use std::io::{Read, Write};
use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub(super) use unix::terminal_size_for_fd;

#[cfg(unix)]
use super::ProcessInputMode;
use super::{
    CliError, TerminalPumpState, feed_terminal_batch_and_pump, feed_terminal_chunk_and_pump,
    task_exited,
};

pub(super) fn run_process_raw_byte_feed_session_after_eval(
    process_stdin: &mut dyn Read,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let policy = pump_state.policy;
    #[cfg(unix)]
    if let ProcessInputMode::PollFd(input_fd) = policy.input_mode {
        return unix::run_process_polled_feed_session_after_eval(
            process_stdin,
            input_fd,
            terminal,
            terminal_id,
            runtime,
            pump_state,
            process_stdout,
        );
    }

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
        feed_terminal_chunk_and_pump(
            terminal,
            terminal_id,
            runtime,
            &byte[..count],
            policy.ready_io_turns,
            policy.event_loop_wait_budget,
            process_stdout,
        )?;
        if task_exited(runtime)? {
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
    process_stdin: &mut dyn Read,
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let policy = pump_state.policy;
    #[cfg(unix)]
    if let ProcessInputMode::PollFd(input_fd) = policy.input_mode {
        return unix::run_process_polled_feed_session_after_eval(
            process_stdin,
            input_fd,
            terminal,
            terminal_id,
            runtime,
            pump_state,
            process_stdout,
        );
    }

    let mut line = Vec::new();
    while read_process_line_after_eval(process_stdin, &mut line)? {
        feed_terminal_batch_and_pump(
            terminal,
            terminal_id,
            runtime,
            &[line.clone()],
            policy.ready_io_turns,
            policy.event_loop_wait_budget,
            process_stdout,
        )?;
        if task_exited(runtime)? {
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
