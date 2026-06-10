//! A hand-driven runner so running/abort/not-finished paths are testable.
//!
//! v0 runs a job synchronously inside the `ctl run` write, so observing a
//! *running* job requires the run to be parked while another thread inspects
//! the device. The simplest honest mechanism for that: [`ManualRunner::run`]
//! blocks on a condvar until the paired [`ManualHandle`] finishes it, or
//! until the run's own [`RunContext`] abort flag is raised (`ctl abort` sets
//! the flag, then the runner's abort hook wakes the condvar). Runs are keyed
//! by the context's job id, so concurrent runs each wait for their own
//! outcome — no single-in-flight workaround.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use serde_json::Value;
use wanix_job::{ErrorKind, JobError};

use crate::{RunContext, RunOutcome, ToolRunner};

#[derive(Debug, Default)]
struct State {
    /// Job ids currently parked inside `run`, in arrival order.
    running: Vec<String>,
    /// Outcomes the handle has assigned, keyed by job id.
    finished: HashMap<String, RunOutcome>,
}

#[derive(Debug, Default)]
struct Shared {
    state: Mutex<State>,
    cond: Condvar,
}

/// The runner half: park inside `run` until the handle decides the outcome
/// or the run is aborted through its context.
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
    fn run(&self, _input: &[u8], _params: Option<&Value>, ctx: &RunContext) -> RunOutcome {
        let Ok(mut state) = self.shared.state.lock() else {
            return poisoned_outcome();
        };
        state.running.push(ctx.job_id().to_owned());
        self.shared.cond.notify_all();
        loop {
            if let Some(outcome) = state.finished.remove(ctx.job_id()) {
                state.running.retain(|id| id != ctx.job_id());
                self.shared.cond.notify_all();
                return outcome;
            }
            if ctx.aborted() {
                state.running.retain(|id| id != ctx.job_id());
                self.shared.cond.notify_all();
                return RunOutcome::failure(
                    JobError::new(ErrorKind::Aborted, "manual runner aborted"),
                    None,
                    Vec::new(),
                );
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

    /// The context abort flag is already set when this hook fires; the
    /// parked run only needs a wakeup to observe it. The state lock is taken
    /// first so the notify cannot slip between a run's flag check and its
    /// wait (the classic missed-wakeup race).
    fn abort(&self, _job_id: &str) {
        let _state = self.shared.state.lock();
        self.shared.cond.notify_all();
    }
}

impl ManualHandle {
    /// Blocks until some run has entered `run`.
    pub fn wait_running(&self) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        while state.running.is_empty() {
            state = match self.shared.cond.wait(state) {
                Ok(state) => state,
                Err(_) => return,
            };
        }
    }

    /// Completes the oldest in-flight run with `outcome`.
    pub fn finish(&self, outcome: RunOutcome) {
        if let Ok(mut state) = self.shared.state.lock() {
            if let Some(id) = state.running.first().cloned() {
                state.finished.insert(id, outcome);
            }
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
