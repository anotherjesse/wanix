use std::time::Duration;

use super::ProcessResizeSource;
#[cfg(all(test, unix))]
use super::resize_queue_source;
#[cfg(unix)]
use super::terminal_size_source;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::qjs_term) enum ProcessInputMode {
    Blocking,
    #[cfg(unix)]
    PollFd(libc::c_int),
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

impl ProcessEventSources {
    pub(in crate::qjs_term) fn blocking() -> Self {
        Self {
            input_mode: ProcessInputMode::Blocking,
            resize_source: ProcessResizeSource::none(),
        }
    }

    #[cfg(unix)]
    pub(in crate::qjs_term) fn input_fd(input_fd: libc::c_int) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: ProcessResizeSource::none(),
        }
    }

    #[cfg(unix)]
    pub(in crate::qjs_term) fn terminal_fds(
        input_fd: libc::c_int,
        terminal_size_fd: libc::c_int,
    ) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: terminal_size_source(terminal_size_fd),
        }
    }

    #[cfg(all(test, unix))]
    pub(in crate::qjs_term) fn resize_queue(
        input_fd: libc::c_int,
        resize_queue: super::ResizeQueue,
    ) -> Self {
        Self {
            input_mode: ProcessInputMode::PollFd(input_fd),
            resize_source: resize_queue_source(resize_queue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ProcessEventSources, ProcessInputMode, TerminalPumpPolicy};
    use crate::qjs_term::pump::TermResize;
    use std::time::Duration;

    #[test]
    fn blocking_event_source_uses_blocking_input_without_resize_source() {
        let mut sources = ProcessEventSources::blocking();

        assert_eq!(sources.input_mode, ProcessInputMode::Blocking);
        assert!(sources.resize_source.is_none());
        assert_eq!(sources.resize_source.next_resize().unwrap(), None);
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
        assert_eq!(sources.resize_source.terminal_size_fd(), Some(4));
    }

    #[cfg(unix)]
    #[test]
    fn input_fd_event_source_polls_input_without_resize_source() {
        let mut sources = ProcessEventSources::input_fd(5);

        assert_eq!(sources.input_mode, ProcessInputMode::PollFd(5));
        assert!(sources.resize_source.is_none());
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
