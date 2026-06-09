//! Interactive `sh`: a detached wasm shell task pumped against the host tty.
//!
//! The guest REPL owns echo, line editing, Ctrl-C, and Ctrl-D (ADR 0003); the
//! host side only moves bytes. The shell task runs detached on its own thread
//! (it blocks reading `#term/<id>/program`), so this loop never waits on the
//! guest: it polls the host stdin fd, feeds bytes into `#term/<id>/data`,
//! forwards terminal-size changes into `winch`, drains terminal output to the
//! process stdout, and finishes when the task records its exit.
//!
//! Host stdin EOF (a script piped into `sh`) is mapped onto the REPL contract
//! instead of inventing a hangup: feed `\r` (complete any pending line) and
//! `\x04` (Ctrl-D), then keep draining until the shell finishes the queued
//! input, in order, and exits.

use std::io::{Read, Write};
use std::time::Duration;

use wanix_task::Task;

use super::{ShCommand, allocate_sh_task, configure_sh_task, sh_namespace};
use crate::qjs_term::{
    AttachedTerminal, NonBlockingFd, ProcessResizeSource, ProcessStdinPoll, ProcessStdinRead,
    attach_task_terminal, drain_terminal_output_bytes, feed_terminal_after_eval,
    feed_terminal_resize_after_eval, poll_process_stdin, read_process_stdin_after_poll,
    terminal_size_source,
};
use crate::{CliError, parse_exit, write_process_output};

const SH_STDIN_READ_CHUNK_BYTES: usize = 1024;
const SH_IDLE_POLL: Duration = Duration::from_millis(20);
const SH_EXIT_DRAIN_POLL: Duration = Duration::from_millis(10);

/// Runs the interactive shell session against a pollable host stdin fd, with
/// terminal resizes read from `terminal_size_fd` when given.
///
/// # Errors
///
/// Returns a CLI error when the session namespace, terminal, or task cannot be
/// built, or when host IO fails. (On an error return the mesh keepalives drop
/// while the detached shell may still run; its next mount op fails inside the
/// task thread, which the detached runner catches and records as an exit.)
pub(crate) fn run_sh_session(
    command: ShCommand,
    stdin_fd: libc::c_int,
    terminal_size_fd: Option<libc::c_int>,
    process_stdin: &mut dyn Read,
    process_stdout: &mut dyn Write,
) -> Result<i32, CliError> {
    // Declared first so the mesh keepalives drop LAST, after the task table.
    let (namespace, _mesh_mounts) = sh_namespace(&command)?;
    let (table, task) = allocate_sh_task(namespace)?;
    let terminal = attach_task_terminal(&task, None)?;
    let mut env = command.env.clone();
    env.push(format!("WANIX_TERM_ID={}", terminal.id));
    configure_sh_task(&task, None, &env)?;

    let mut resize = match terminal_size_fd {
        Some(fd) => terminal_size_source(fd),
        None => ProcessResizeSource::none(),
    };
    // Seed the current size before the shell starts.
    forward_resize(&terminal, &mut resize)?;
    table.start_detached(task.id())?;

    let _nonblocking = NonBlockingFd::enter(stdin_fd)?;
    let mut bytes = [0u8; SH_STDIN_READ_CHUNK_BYTES];
    loop {
        forward_resize(&terminal, &mut resize)?;
        if let ProcessStdinPoll::Ready = poll_process_stdin(stdin_fd, SH_IDLE_POLL)? {
            match read_process_stdin_after_poll(process_stdin, &mut bytes)? {
                ProcessStdinRead::Bytes(count) => {
                    feed_terminal_after_eval(&terminal.device, &terminal.id, &bytes[..count])?;
                }
                ProcessStdinRead::Eof => {
                    feed_terminal_after_eval(&terminal.device, &terminal.id, b"\r\x04")?;
                    return drain_until_exit(&task, &terminal, process_stdout);
                }
                ProcessStdinRead::Interrupted | ProcessStdinRead::Idle => {}
            }
        }
        drain_to_stdout(&terminal, process_stdout)?;
        if !task.exit().is_empty() {
            return finish_session(&task, &terminal, process_stdout);
        }
    }
}

fn forward_resize(
    terminal: &AttachedTerminal,
    resize: &mut ProcessResizeSource,
) -> Result<(), CliError> {
    let Some(resize) = resize.next_resize()? else {
        return Ok(());
    };
    feed_terminal_resize_after_eval(&terminal.device, &terminal.id, &resize)
}

/// After host input ended: keep draining until the shell processes the queued
/// bytes (which now end in Ctrl-D) and records its exit.
fn drain_until_exit(
    task: &Task,
    terminal: &AttachedTerminal,
    process_stdout: &mut dyn Write,
) -> Result<i32, CliError> {
    loop {
        drain_to_stdout(terminal, process_stdout)?;
        if !task.exit().is_empty() {
            return finish_session(task, terminal, process_stdout);
        }
        std::thread::sleep(SH_EXIT_DRAIN_POLL);
    }
}

fn finish_session(
    task: &Task,
    terminal: &AttachedTerminal,
    process_stdout: &mut dyn Write,
) -> Result<i32, CliError> {
    drain_to_stdout(terminal, process_stdout)?;
    Ok(parse_exit(&task.exit()))
}

fn drain_to_stdout(
    terminal: &AttachedTerminal,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let output = drain_terminal_output_bytes(&terminal.device, &terminal.id)?;
    if output.is_empty() {
        return Ok(());
    }
    write_process_output(process_stdout, "stdout", &output)
}
