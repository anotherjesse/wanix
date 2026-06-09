#[cfg(all(test, unix))]
use std::collections::VecDeque;
#[cfg(all(test, unix))]
use std::sync::{Arc, Mutex};

use super::super::CliError;
#[cfg(unix)]
use super::super::process::terminal_size_for_fd;

#[cfg(all(test, unix))]
pub(in crate::qjs_term) type ResizeQueue = Arc<Mutex<VecDeque<(u16, u16)>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TermResize {
    pub(crate) columns: u16,
    pub(crate) rows: u16,
}

use std::fmt;

type ResizePollFn = fn(&mut ResizeSourceState) -> Result<Option<TermResize>, CliError>;

#[derive(Clone)]
pub(crate) struct ProcessResizeSource {
    kind: &'static str,
    state: ResizeSourceState,
    poll: ResizePollFn,
}

#[derive(Clone)]
enum ResizeSourceState {
    None,
    #[cfg(unix)]
    TerminalSizeFd(TerminalSizeSource),
    #[cfg(all(test, unix))]
    Queue(ResizeQueue),
}

impl ProcessResizeSource {
    pub(crate) fn none() -> Self {
        Self {
            kind: "none",
            state: ResizeSourceState::None,
            poll: poll_no_resize,
        }
    }

    pub(crate) fn next_resize(&mut self) -> Result<Option<TermResize>, CliError> {
        (self.poll)(&mut self.state)
    }

    #[cfg(test)]
    pub(in crate::qjs_term) fn is_none(&self) -> bool {
        matches!(self.state, ResizeSourceState::None)
    }

    #[cfg(all(test, unix))]
    pub(in crate::qjs_term) fn terminal_size_fd(&self) -> Option<libc::c_int> {
        match &self.state {
            ResizeSourceState::TerminalSizeFd(source) => Some(source.fd),
            _ => None,
        }
    }
}

impl fmt::Debug for ProcessResizeSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessResizeSource")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::qjs_term) struct TerminalSizeSource {
    fd: libc::c_int,
    last: Option<TermResize>,
}

#[cfg(unix)]
pub(crate) fn terminal_size_source(fd: libc::c_int) -> ProcessResizeSource {
    ProcessResizeSource {
        kind: "terminal-size-fd",
        state: ResizeSourceState::TerminalSizeFd(TerminalSizeSource::new(fd)),
        poll: poll_terminal_size,
    }
}

#[cfg(all(test, unix))]
pub(in crate::qjs_term) fn resize_queue_source(queue: ResizeQueue) -> ProcessResizeSource {
    ProcessResizeSource {
        kind: "queue",
        state: ResizeSourceState::Queue(queue),
        poll: poll_resize_queue,
    }
}

fn poll_no_resize(_state: &mut ResizeSourceState) -> Result<Option<TermResize>, CliError> {
    Ok(None)
}

#[cfg(unix)]
fn poll_terminal_size(state: &mut ResizeSourceState) -> Result<Option<TermResize>, CliError> {
    let ResizeSourceState::TerminalSizeFd(source) = state else {
        return Ok(None);
    };
    source.next_resize()
}

#[cfg(all(test, unix))]
fn poll_resize_queue(state: &mut ResizeSourceState) -> Result<Option<TermResize>, CliError> {
    let ResizeSourceState::Queue(queue) = state else {
        return Ok(None);
    };
    next_queued_resize(queue)
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

#[cfg(all(test, unix))]
fn next_queued_resize(queue: &ResizeQueue) -> Result<Option<TermResize>, CliError> {
    let Some((columns, rows)) = queue
        .lock()
        .map_err(|_| CliError::new("test resize queue lock poisoned", 1))?
        .pop_front()
    else {
        return Ok(None);
    };
    Ok(Some(TermResize { columns, rows }))
}
