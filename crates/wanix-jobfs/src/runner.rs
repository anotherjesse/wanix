//! The runner seam: host policy behind one job device.
//!
//! A [`JobRunner`] is the host-chosen operation. The filesystem contract,
//! quotas, and lifecycle live in [`crate::JobCore`]; the runner sees only
//! sealed input bytes, validated params, and its per-run [`RunContext`], and
//! returns a [`RunOutcome`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use wanix_fs::LineBuffer;
use wanix_job::JobError;

/// What one runner invocation produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOutcome {
    /// Primary output bytes (the job's `out` file).
    pub out: Vec<u8>,
    /// Diagnostic bytes (the job's `err` file).
    pub err: Vec<u8>,
    /// Runner exit code, when the runner has one.
    pub exit_code: Option<i32>,
    /// The failure, when the run did not succeed.
    pub error: Option<JobError>,
}

impl RunOutcome {
    /// A successful run: output bytes, exit code 0, no error.
    #[must_use]
    pub fn success(out: Vec<u8>) -> Self {
        Self {
            out,
            err: Vec::new(),
            exit_code: Some(0),
            error: None,
        }
    }

    /// A failed run: a taxonomy error plus optional exit code and diagnostics.
    #[must_use]
    pub fn failure(error: JobError, exit_code: Option<i32>, err: Vec<u8>) -> Self {
        Self {
            out: Vec::new(),
            err,
            exit_code,
            error: Some(error),
        }
    }
}

/// Per-run context handed to [`JobRunner::run`]: the job's identity, time
/// budget, live abort flag, and progress sink.
///
/// The abort flag is the job's own (`ctl abort` sets it), so cancellation is
/// correlatable to the exact run; the progress sink feeds the job's `events`
/// stream file (bounded, lossy, never-EOF until the job finishes).
#[derive(Clone)]
pub struct RunContext {
    job_id: String,
    deadline: Option<u64>,
    abort: Arc<AtomicBool>,
    progress: Arc<LineBuffer>,
}

impl RunContext {
    /// A context bound to one job's abort flag and events buffer
    /// (constructed by the device core at `ctl run`).
    #[must_use]
    pub fn new(
        job_id: String,
        deadline: Option<u64>,
        abort: Arc<AtomicBool>,
        progress: Arc<LineBuffer>,
    ) -> Self {
        Self {
            job_id,
            deadline,
            abort,
            progress,
        }
    }

    /// A free-standing context for direct runner invocations (unit tests,
    /// one-shot helpers): no deadline, a private abort flag, a private
    /// default-bounded events buffer.
    #[must_use]
    pub fn detached(job_id: impl Into<String>) -> Self {
        Self::new(
            job_id.into(),
            None,
            Arc::new(AtomicBool::new(false)),
            Arc::new(LineBuffer::default()),
        )
    }

    /// The opaque id of the job this run belongs to.
    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    /// Absolute run deadline in Unix-epoch milliseconds, when the device's
    /// `runTimeoutMs` declares a budget. A runner should stop work past it;
    /// the core's finalize backstop records `timeout` even if it does not.
    #[must_use]
    pub fn deadline(&self) -> Option<u64> {
        self.deadline
    }

    /// Whether `ctl abort` has been requested for this job. A cooperative
    /// runner polls this at its work boundaries and returns early.
    #[must_use]
    pub fn aborted(&self) -> bool {
        self.abort.load(Ordering::SeqCst)
    }

    /// Publishes progress bytes to the job's `events` stream (callers send
    /// newline-terminated lines). Bounded drop-oldest, so a chatty runner
    /// never grows host memory; a job with no `events` reader loses the
    /// oldest backlog, never blocks.
    pub fn progress(&self, bytes: &[u8]) {
        self.progress.push(bytes);
    }
}

impl std::fmt::Debug for RunContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunContext")
            .field("job_id", &self.job_id)
            .field("deadline", &self.deadline)
            .field("aborted", &self.aborted())
            .finish_non_exhaustive()
    }
}

/// A synchronous job runner.
///
/// `run` is invoked on the caller's thread (inside the `ctl run` write) and
/// **never** while the core holds its job-table lock, so a runner may block,
/// and `abort` may be invoked concurrently from another file handle.
pub trait JobRunner: Send + Sync {
    /// Runs sealed input and committed params to completion.
    fn run(&self, input: &[u8], params: Option<&Value>, ctx: &RunContext) -> RunOutcome;

    /// Whether [`JobRunner::abort`] can interrupt a run in progress.
    /// Defaults to `false`; abort then only marks the job's final state.
    fn abort_supported(&self) -> bool {
        false
    }

    /// Best-effort cancellation hook for a running job, invoked *after* the
    /// job's abort flag ([`RunContext::aborted`]) is set: wake or interrupt
    /// the run however the runner can. Default: no-op.
    fn abort(&self, _job_id: &str) {}
}
