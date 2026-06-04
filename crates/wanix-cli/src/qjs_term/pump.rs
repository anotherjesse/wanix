#[cfg(all(test, unix))]
use std::collections::VecDeque;
use std::io::Write;
#[cfg(all(test, unix))]
use std::sync::Arc;
#[cfg(all(test, unix))]
use std::sync::Mutex;
use std::time::Duration;

use wanix_fs::{FileSystem, NormalizedPath, OpenOptions};
use wanix_qjs::QuickJsTaskRuntime;
use wanix_term::TermDevice;

use crate::write_process_output;

use super::CliError;
#[cfg(unix)]
use super::process::terminal_size_for_fd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TermResize {
    pub(super) columns: u16,
    pub(super) rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProcessInputMode {
    Blocking,
    #[cfg(unix)]
    PollFd(libc::c_int),
}

#[derive(Debug, Clone)]
pub(super) enum ProcessResizeSource {
    None,
    #[cfg(unix)]
    TerminalSizeFd(TerminalSizeSource),
    #[cfg(all(test, unix))]
    Queue(Arc<Mutex<VecDeque<(u16, u16)>>>),
}

#[derive(Debug, Clone)]
pub(super) struct ProcessEventSources {
    pub(super) input_mode: ProcessInputMode,
    pub(super) resize_source: ProcessResizeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TerminalPumpPolicy {
    pub(super) ready_io_turns: usize,
    pub(super) event_loop_wait_budget: Duration,
    pub(super) input_mode: ProcessInputMode,
}

#[derive(Debug, Clone)]
pub(super) struct TerminalPumpState {
    pub(super) policy: TerminalPumpPolicy,
    pub(super) resize_source: ProcessResizeSource,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TerminalSizeSource {
    fd: libc::c_int,
    last: Option<TermResize>,
}

impl ProcessResizeSource {
    pub(super) fn next_resize(&mut self) -> Result<Option<TermResize>, CliError> {
        match self {
            Self::None => Ok(None),
            #[cfg(unix)]
            Self::TerminalSizeFd(source) => source.next_resize(),
            #[cfg(all(test, unix))]
            Self::Queue(queue) => {
                let Some((columns, rows)) = queue
                    .lock()
                    .map_err(|_| CliError::new("test resize queue lock poisoned", 1))?
                    .pop_front()
                else {
                    return Ok(None);
                };
                Ok(Some(TermResize { columns, rows }))
            }
        }
    }
}

impl ProcessEventSources {
    pub(super) fn blocking() -> Self {
        Self {
            input_mode: ProcessInputMode::Blocking,
            resize_source: ProcessResizeSource::None,
        }
    }

    #[cfg(unix)]
    pub(super) fn input_fd(input_fd: libc::c_int) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::None,
        }
    }

    #[cfg(unix)]
    pub(super) fn terminal_fds(input_fd: libc::c_int, terminal_size_fd: libc::c_int) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::TerminalSizeFd(TerminalSizeSource::new(
                terminal_size_fd,
            )),
        }
    }

    #[cfg(all(test, unix))]
    pub(super) fn resize_queue(
        input_fd: libc::c_int,
        resize_queue: Arc<Mutex<VecDeque<(u16, u16)>>>,
    ) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::Queue(resize_queue),
        }
    }
}

#[cfg(unix)]
impl TerminalSizeSource {
    fn new(fd: libc::c_int) -> Self {
        Self { fd, last: None }
    }

    fn next_resize(&mut self) -> Result<Option<TermResize>, CliError> {
        let Some(resize) = terminal_size_for_fd(self.fd)? else {
            return Ok(None);
        };
        if self.last == Some(resize) {
            return Ok(None);
        }
        self.last = Some(resize);
        Ok(Some(resize))
    }
}

impl TermResize {
    pub(super) fn payload(&self) -> Vec<u8> {
        format!("{} {}\n", self.columns, self.rows).into_bytes()
    }
}

pub(super) fn flush_terminal_feed_batch(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    batch: &mut Vec<Vec<u8>>,
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    if batch.is_empty() {
        return Ok(());
    }
    let flushed = std::mem::take(batch);
    feed_terminal_batch_and_pump(
        terminal,
        terminal_id,
        runtime,
        &flushed,
        ready_io_turns,
        event_loop_wait_budget,
        process_stdout,
    )
}

pub(super) fn feed_terminal_batch_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    batch: &[Vec<u8>],
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        for chunk in batch {
            feed_terminal_after_eval(terminal, terminal_id, chunk)?;
        }
        runtime.run_event_loop_turns(event_loop_wait_budget, ready_io_turns)?;
        Ok(())
    })();
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

pub(super) fn feed_terminal_chunk_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    chunk: &[u8],
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        feed_terminal_after_eval(terminal, terminal_id, chunk)?;
        runtime.run_event_loop_turns(event_loop_wait_budget, ready_io_turns)?;
        Ok(())
    })();
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

pub(super) fn pump_terminal_idle(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = runtime
        .run_event_loop_turns(event_loop_wait_budget, ready_io_turns)
        .map_err(CliError::from);
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

pub(super) fn pump_terminal_resize_if_changed(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    pump_state: &mut TerminalPumpState,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let Some(resize) = pump_state.resize_source.next_resize()? else {
        return Ok(());
    };
    feed_terminal_resize_and_pump(
        terminal,
        terminal_id,
        runtime,
        &resize,
        pump_state.policy.ready_io_turns,
        pump_state.policy.event_loop_wait_budget,
        process_stdout,
    )
}

pub(super) fn feed_terminal_resize_and_pump(
    terminal: &TermDevice,
    terminal_id: &str,
    runtime: &mut QuickJsTaskRuntime,
    resize: &TermResize,
    ready_io_turns: usize,
    event_loop_wait_budget: Duration,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    let result = (|| -> Result<(), CliError> {
        feed_terminal_resize_after_eval(terminal, terminal_id, resize)?;
        runtime.run_event_loop_turns(event_loop_wait_budget, ready_io_turns)?;
        Ok(())
    })();
    drain_terminal_output(terminal, terminal_id, process_stdout)?;
    result
}

pub(super) fn task_exited(runtime: &QuickJsTaskRuntime) -> Result<bool, CliError> {
    Ok(runtime.exit_code()?.is_some())
}

pub(super) fn feed_terminal_after_eval(
    terminal: &TermDevice,
    terminal_id: &str,
    chunk: &[u8],
) -> Result<(), CliError> {
    if chunk.is_empty() {
        return Ok(());
    }
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    data.write(chunk)?;
    Ok(())
}

pub(super) fn feed_terminal_resize_after_eval(
    terminal: &TermDevice,
    terminal_id: &str,
    resize: &TermResize,
) -> Result<(), CliError> {
    let mut winch = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/winch"))?,
        OpenOptions {
            write: true,
            ..OpenOptions::default()
        },
    )?;
    winch.write(&resize.payload())?;
    Ok(())
}

pub(super) fn drain_terminal_output(
    terminal: &TermDevice,
    terminal_id: &str,
    process_stdout: &mut dyn Write,
) -> Result<(), CliError> {
    write_process_output(
        process_stdout,
        "stdout",
        &drain_terminal_output_bytes(terminal, terminal_id)?,
    )
}

pub(super) fn drain_terminal_output_bytes(
    terminal: &TermDevice,
    terminal_id: &str,
) -> Result<Vec<u8>, CliError> {
    let mut data = terminal.open(
        &NormalizedPath::new(format!("{terminal_id}/data"))?,
        OpenOptions {
            read: true,
            ..OpenOptions::default()
        },
    )?;
    let mut output = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let count = data.read(&mut buf)?;
        if count == 0 {
            return Ok(output);
        }
        output.extend_from_slice(&buf[..count]);
    }
}
