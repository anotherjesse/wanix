//! A hand-driven runner so running/abort/not-finished paths are testable.
//!
//! v0 runs a job synchronously inside the `ctl run` write, so observing a
//! *running* job requires the run to be parked while another thread inspects
//! the device. The simplest honest mechanism for that: [`ManualRunner::run`]
//! blocks on a condvar until the paired [`ManualHandle`] finishes it (or
//! `ctl abort` interrupts it through the runner's abort hook). One run at a
//! time: the handle drives the single in-flight invocation.

use std::sync::{Arc, Condvar, Mutex};

use serde_json::Value;
use wanix_job::{ErrorKind, JobError};

use crate::runner::{RunOutcome, ToolRunner};

#[derive(Debug, Default)]
struct State {
    running: bool,
    command: Option<Command>,
}

#[derive(Debug)]
enum Command {
    Finish(RunOutcome),
    Abort,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    cond: Condvar,
}

/// The runner half: park inside `run` until the handle decides the outcome.
#[derive(Debug, Clone)]
pub struct ManualRunner {
    shared: Arc<Shared>,
}

/// The test/driver half: observe that a run started and decide its outcome.
#[derive(Debug, Clone)]
pub struct ManualHandle {
    shared: Arc<Shared>,
}

impl ManualRunner {
    /// A fresh runner plus the handle that drives it.
    #[must_use]
    pub fn new() -> (Self, ManualHandle) {
        let shared = Arc::new(Shared::default());
        (
            Self {
                shared: Arc::clone(&shared),
            },
            ManualHandle { shared },
        )
    }
}

impl ToolRunner for ManualRunner {
    fn run(&self, _input: &[u8], _params: Option<&Value>) -> RunOutcome {
        let Ok(mut state) = self.shared.state.lock() else {
            return poisoned_outcome();
        };
        state.running = true;
        self.shared.cond.notify_all();
        loop {
            if let Some(command) = state.command.take() {
                state.running = false;
                self.shared.cond.notify_all();
                return match command {
                    Command::Finish(outcome) => outcome,
                    Command::Abort => RunOutcome::failure(
                        JobError::new(ErrorKind::Aborted, "manual runner aborted"),
                        None,
                        Vec::new(),
                    ),
                };
            }
            state = match self.shared.cond.wait(state) {
                Ok(state) => state,
                Err(_) => return poisoned_outcome(),
            };
        }
    }

    fn abort_supported(&self) -> bool {
        true
    }

    fn abort(&self, _job_id: &str) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.command = Some(Command::Abort);
            self.shared.cond.notify_all();
        }
    }
}

impl ManualHandle {
    /// Blocks until the runner has entered `run`.
    pub fn wait_running(&self) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        while !state.running {
            state = match self.shared.cond.wait(state) {
                Ok(state) => state,
                Err(_) => return,
            };
        }
    }

    /// Completes the in-flight run with `outcome`.
    pub fn finish(&self, outcome: RunOutcome) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.command = Some(Command::Finish(outcome));
            self.shared.cond.notify_all();
        }
    }
}

fn poisoned_outcome() -> RunOutcome {
    RunOutcome::failure(
        JobError::new(ErrorKind::Internal, "manual runner lock poisoned"),
        None,
        Vec::new(),
    )
}
