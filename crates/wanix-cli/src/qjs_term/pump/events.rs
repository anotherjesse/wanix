#[cfg(all(test, unix))]
use std::collections::VecDeque;
#[cfg(all(test, unix))]
use std::sync::Arc;
#[cfg(all(test, unix))]
use std::sync::Mutex;
use std::time::Duration;

use super::super::CliError;
#[cfg(unix)]
use super::super::process::terminal_size_for_fd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::qjs_term) struct TermResize {
    pub(in crate::qjs_term) columns: u16,
    pub(in crate::qjs_term) rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::qjs_term) enum ProcessInputMode {
    Blocking,
    #[cfg(unix)]
    PollFd(libc::c_int),
}

#[derive(Debug, Clone)]
pub(in crate::qjs_term) enum ProcessResizeSource {
    None,
    #[cfg(unix)]
    TerminalSizeFd(TerminalSizeSource),
    #[cfg(all(test, unix))]
    Queue(Arc<Mutex<VecDeque<(u16, u16)>>>),
}

#[derive(Debug, Clone)]
pub(in crate::qjs_term) struct ProcessEventSources {
    pub(in crate::qjs_term) input_mode: ProcessInputMode,
    pub(in crate::qjs_term) resize_source: ProcessResizeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::qjs_term) struct TerminalPumpPolicy {
    pub(in crate::qjs_term) ready_io_turns: usize,
    pub(in crate::qjs_term) event_loop_wait_budget: Duration,
    pub(in crate::qjs_term) input_mode: ProcessInputMode,
}

#[derive(Debug, Clone)]
pub(in crate::qjs_term) struct TerminalPumpState {
    pub(in crate::qjs_term) policy: TerminalPumpPolicy,
    pub(in crate::qjs_term) resize_source: ProcessResizeSource,
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::qjs_term) struct TerminalSizeSource {
    fd: libc::c_int,
    last: Option<TermResize>,
}

impl ProcessResizeSource {
    pub(in crate::qjs_term) fn next_resize(&mut self) -> Result<Option<TermResize>, CliError> {
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
    pub(in crate::qjs_term) fn blocking() -> Self {
        Self {
            input_mode: ProcessInputMode::Blocking,
            resize_source: ProcessResizeSource::None,
        }
    }

    #[cfg(unix)]
    pub(in crate::qjs_term) fn input_fd(input_fd: libc::c_int) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::None,
        }
    }

    #[cfg(unix)]
    pub(in crate::qjs_term) fn terminal_fds(
        input_fd: libc::c_int,
        terminal_size_fd: libc::c_int,
    ) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::TerminalSizeFd(TerminalSizeSource::new(
                terminal_size_fd,
            )),
        }
    }

    #[cfg(all(test, unix))]
    pub(in crate::qjs_term) fn resize_queue(
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
    pub(in crate::qjs_term) fn payload(&self) -> Vec<u8> {
        format!("{} {}\n", self.columns, self.rows).into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ProcessEventSources, ProcessInputMode, ProcessResizeSource, TermResize, TerminalPumpPolicy,
    };
    use std::time::Duration;

    #[test]
    fn blocking_event_source_uses_blocking_input_without_resize_source() {
        let mut sources = ProcessEventSources::blocking();

        assert_eq!(sources.input_mode, ProcessInputMode::Blocking);
        assert!(matches!(sources.resize_source, ProcessResizeSource::None));
        assert_eq!(sources.resize_source.next_resize().unwrap(), None);
    }

    #[test]
    fn terminal_resize_payload_matches_winch_contract() {
        let resize = TermResize {
            columns: 132,
            rows: 43,
        };

        assert_eq!(resize.payload(), b"132 43\n");
    }

    #[test]
    fn terminal_pump_policy_carries_input_mode_and_budgets() {
        let policy = TerminalPumpPolicy {
            ready_io_turns: 7,
            event_loop_wait_budget: Duration::from_millis(25),
            input_mode: ProcessInputMode::Blocking,
        };

        assert_eq!(policy.ready_io_turns, 7);
        assert_eq!(policy.event_loop_wait_budget, Duration::from_millis(25));
        assert_eq!(policy.input_mode, ProcessInputMode::Blocking);
    }

    #[cfg(unix)]
    #[test]
    fn terminal_fds_event_source_polls_input_and_tracks_terminal_size_fd() {
        let sources = ProcessEventSources::terminal_fds(3, 4);

        assert_eq!(sources.input_mode, ProcessInputMode::PollFd(3));
        match sources.resize_source {
            ProcessResizeSource::TerminalSizeFd(source) => {
                assert_eq!(source.fd, 4);
                assert_eq!(source.last, None);
            }
            other => panic!("unexpected resize source: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn input_fd_event_source_polls_input_without_resize_source() {
        let mut sources = ProcessEventSources::input_fd(5);

        assert_eq!(sources.input_mode, ProcessInputMode::PollFd(5));
        assert!(matches!(sources.resize_source, ProcessResizeSource::None));
        assert_eq!(sources.resize_source.next_resize().unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn queued_resize_source_returns_resizes_in_order() {
        let resize_queue = std::sync::Arc::new(std::sync::Mutex::new(
            [(80, 24), (100, 30)].into_iter().collect(),
        ));
        let mut sources = ProcessEventSources::resize_queue(9, resize_queue);

        assert_eq!(sources.input_mode, ProcessInputMode::PollFd(9));
        assert_eq!(
            sources.resize_source.next_resize().unwrap(),
            Some(TermResize {
                columns: 80,
                rows: 24
            })
        );
        assert_eq!(
            sources.resize_source.next_resize().unwrap(),
            Some(TermResize {
                columns: 100,
                rows: 30
            })
        );
        assert_eq!(sources.resize_source.next_resize().unwrap(), None);
    }
}
