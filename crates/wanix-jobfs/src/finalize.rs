//! Terminal-state recording for [`JobCore`]: pre-run validation, the
//! finalize step (abort/timeout/output-cap policy), and the pure
//! state-transition helpers.

use std::sync::atomic::Ordering;

use wanix_fs::{FsError, FsResult};
use wanix_job::{ErrorKind, JobError, JobResult, JobState};

use crate::jobs::JobCore;
use crate::runner::RunOutcome;
use crate::table::JobRecord;

impl JobCore {
    pub(crate) fn pre_run_error(
        &self,
        job: &JobRecord,
        principal_bytes: u64,
        running: u64,
    ) -> Option<JobError> {
        let policy = self.policy();
        if policy.params_required && job.params.is_none() && job.params_raw.is_empty() {
            return Some(JobError::new(
                ErrorKind::InvalidParams,
                "params.json is required",
            ));
        }
        if !job.params_raw.is_empty() && job.params.is_none() {
            return Some(JobError::new(
                ErrorKind::InvalidParams,
                "params.json is incomplete JSON",
            ));
        }
        if job.input.len() as u64 > policy.max_input_bytes {
            return Some(JobError::new(
                ErrorKind::InputTooLarge,
                format!("input exceeds maxBytes {}", policy.max_input_bytes),
            ));
        }
        if running >= policy.limits.max_concurrent_per_principal {
            return Some(JobError::new(
                ErrorKind::QuotaExceeded,
                "maxConcurrentPerPrincipal reached",
            ));
        }
        if principal_bytes > policy.limits.max_bytes_per_principal {
            return Some(JobError::new(
                ErrorKind::QuotaExceeded,
                "maxBytesPerPrincipal exceeded",
            ));
        }
        None
    }

    /// Moves a job to its terminal state, records the retained result, and
    /// closes the `events` stream. Verdict precedence: an abort request wins
    /// over whatever the runner returned; then the runner's own error; then
    /// the timeout backstop (a runner that ignored its deadline still records
    /// `timeout`); then the output caps.
    ///
    /// Output-cap policy (`maxOutBytes`/`maxErrBytes`, and `headroom` — the
    /// remaining aggregate `maxTotalBytes` budget): stored bytes are always
    /// clamped, so the memory bound holds whatever the runner produced, and a
    /// run that would otherwise have succeeded records `runner_failed` (a
    /// per-stream cap is the runner's declared contract) or `quota_exceeded`
    /// (the aggregate bound) instead of silently truncating — an explicit
    /// failure beats corrupt-looking output.
    pub(crate) fn finalize(
        &self,
        job: &mut JobRecord,
        now: u64,
        mut outcome: RunOutcome,
        headroom: u64,
    ) {
        let limits = &self.policy().limits;
        let stream_cap_error = clamp_outputs(&mut outcome, limits, headroom);
        let timed_out = self
            .deadline_after(job.started_at.unwrap_or(now))
            .is_some_and(|deadline| now > deadline);
        let (state, error) = if job.abort.load(Ordering::SeqCst) {
            (
                JobState::Aborted,
                Some(JobError::new(ErrorKind::Aborted, "aborted by caller")),
            )
        } else if let Some(error) = outcome.error.take() {
            (JobState::Failed, Some(error))
        } else if timed_out {
            (
                JobState::Failed,
                Some(JobError::new(
                    ErrorKind::Timeout,
                    format!("run exceeded runTimeoutMs {}", limits.run_timeout_ms),
                )),
            )
        } else if let Some(error) = stream_cap_error {
            (JobState::Failed, Some(error))
        } else {
            (JobState::Done, None)
        };
        if transition(job, state, "job").is_err() {
            return;
        }
        job.out = outcome.out;
        job.err = outcome.err;
        job.finished_at = Some(now);
        job.events.close();
        let retain = match state {
            JobState::Done => self.policy().lifecycle.retain_done_ms,
            _ => self.policy().lifecycle.retain_failed_ms,
        };
        job.expires_at = Some(now.saturating_add(retain));
        let retryable = match &error {
            None => self.policy().retryable,
            Some(error) => error.kind.default_retryable(),
        };
        job.result = Some(JobResult {
            state,
            exit_code: outcome.exit_code,
            duration_ms: job.started_at.map(|started| now.saturating_sub(started)),
            input_bytes: job.input.len() as u64,
            output_bytes: job.out.len() as u64,
            error,
            retryable,
        });
    }

    /// The absolute run deadline for a run started at `started`, when the
    /// policy declares a time budget (`run_timeout_ms` of 0 means unlimited).
    /// Feeds both the `RunContext` deadline and the finalize backstop.
    pub(crate) fn deadline_after(&self, started: u64) -> Option<u64> {
        let timeout = self.policy().limits.run_timeout_ms;
        (timeout > 0).then(|| started.saturating_add(timeout))
    }
}

/// Clamps a finished run's streams to the per-stream caps and the aggregate
/// headroom, returning the cap error an otherwise-successful run records.
fn clamp_outputs(
    outcome: &mut RunOutcome,
    limits: &crate::policy::JobLimits,
    headroom: u64,
) -> Option<JobError> {
    let out_len = outcome.out.len() as u64;
    let err_len = outcome.err.len() as u64;
    outcome
        .out
        .truncate(as_len(limits.max_out_bytes.min(headroom)));
    let remaining = headroom.saturating_sub(outcome.out.len() as u64);
    outcome
        .err
        .truncate(as_len(limits.max_err_bytes.min(remaining)));
    if out_len > limits.max_out_bytes {
        return Some(JobError::new(
            ErrorKind::RunnerFailed,
            format!(
                "primary output exceeds maxOutBytes {} (stored bytes truncated)",
                limits.max_out_bytes
            ),
        ));
    }
    if err_len > limits.max_err_bytes {
        return Some(JobError::new(
            ErrorKind::RunnerFailed,
            format!(
                "diagnostics exceed maxErrBytes {} (stored bytes truncated)",
                limits.max_err_bytes
            ),
        ));
    }
    if out_len.saturating_add(err_len) > headroom {
        return Some(JobError::new(
            ErrorKind::QuotaExceeded,
            "storing outputs exceeds maxTotalBytes across all callers (stored bytes truncated)",
        ));
    }
    None
}

fn as_len(bytes: u64) -> usize {
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

/// Validates a state change through `wanix_job`'s pure transition relation.
pub(crate) fn transition(job: &mut JobRecord, next: JobState, id: &str) -> FsResult<()> {
    if !job.state.can_transition_to(next) {
        return Err(FsError::Other(format!(
            "illegal job transition for {id}: {} -> {}",
            job.state.as_str(),
            next.as_str()
        )));
    }
    job.state = next;
    Ok(())
}

/// Input/params may only change before `run` seals them.
pub(crate) fn ensure_receiving(job: &mut JobRecord, id: &str) -> FsResult<()> {
    match job.state {
        JobState::Allocated => transition(job, JobState::Receiving, id),
        JobState::Receiving => Ok(()),
        _ => Err(FsError::Other(format!("job {id} input is sealed"))),
    }
}
