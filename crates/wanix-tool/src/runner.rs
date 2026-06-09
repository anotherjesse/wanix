//! The runner seam: host policy behind one ToolFS.
//!
//! A [`ToolRunner`] is the host-chosen operation. The filesystem contract,
//! quotas, and lifecycle live in [`crate::ToolService`]; the runner sees only
//! sealed input bytes and validated params and returns a [`RunOutcome`].
//! Process execution is out of scope for this crate (`docs/toolfs.md`
//! §"Runner Boundary"); v0 ships deterministic in-process runners in
//! [`crate::runners`].

use serde_json::Value;
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

/// A synchronous tool runner.
///
/// `run` is invoked on the caller's thread (inside the `ctl run` write) and
/// **never** while the service holds its job-table lock, so a runner may
/// block, and `abort` may be invoked concurrently from another file handle.
pub trait ToolRunner: Send + Sync {
    /// Runs sealed input and committed params to completion.
    fn run(&self, input: &[u8], params: Option<&Value>) -> RunOutcome;

    /// Whether [`ToolRunner::abort`] can interrupt a run in progress.
    /// Defaults to `false`; abort then only marks the job's final state.
    fn abort_supported(&self) -> bool {
        false
    }

    /// Best-effort cancellation hook for a running job. Default: no-op.
    fn abort(&self, _job_id: &str) {}
}
