//! The effectful half of [`ToolService`]: input/params mutation and the
//! `ctl` verbs (`run`, `abort`, `close`), with job-state changes validated
//! through `wanix_job`'s pure transition relation. The runner is always
//! invoked with the job-table lock released.

use std::sync::Arc;

use serde_json::Value;
use wanix_fs::{FsError, FsResult};
use wanix_job::{ErrorKind, JobError, JobResult, JobState};

use super::{JobField, ToolService};
use crate::jobs::JobRecord;
use crate::principal::ToolPrincipal;
use crate::runner::RunOutcome;

impl ToolService {
    /// Appends bytes to a job's `in`. Sealed once `ctl run` is accepted.
    /// Growth is bounded by the table-wide byte cap as the bytes arrive.
    pub(crate) fn append_input(
        &self,
        principal: &ToolPrincipal,
        id: &str,
        bytes: &[u8],
    ) -> FsResult<usize> {
        let mut table = self.locked(self.now())?;
        self.ensure_total_bytes_headroom(table.total_bytes(), bytes.len())?;
        let job = table.get_mut(principal, id)?;
        ensure_receiving(job, id)?;
        job.input.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    /// Appends bytes to a job's `params.json`, parsing at write time: a full
    /// JSON value commits, a prefix of one keeps buffering, anything else is
    /// rejected immediately.
    pub(crate) fn append_params(
        &self,
        principal: &ToolPrincipal,
        id: &str,
        bytes: &[u8],
    ) -> FsResult<usize> {
        let mut table = self.locked(self.now())?;
        self.ensure_total_bytes_headroom(table.total_bytes(), bytes.len())?;
        let job = table.get_mut(principal, id)?;
        ensure_receiving(job, id)?;
        let mut tentative = job.params_raw.clone();
        tentative.extend_from_slice(bytes);
        match serde_json::from_slice::<Value>(&tentative) {
            Ok(value) => {
                job.params_raw = tentative;
                job.params = Some(value);
            }
            Err(err) if err.is_eof() => {
                job.params_raw = tentative;
                job.params = None;
            }
            Err(err) => {
                return Err(FsError::Other(format!(
                    "params.json is not valid JSON: {err}"
                )));
            }
        }
        Ok(bytes.len())
    }

    /// Clears a job's `in` or `params.json` (an open with `truncate`).
    pub(crate) fn reset_field(
        &self,
        principal: &ToolPrincipal,
        id: &str,
        field: JobField,
    ) -> FsResult<()> {
        let mut table = self.locked(self.now())?;
        let job = table.get_mut(principal, id)?;
        ensure_receiving(job, id)?;
        match field {
            JobField::In => job.input.clear(),
            JobField::Params => {
                job.params_raw.clear();
                job.params = None;
            }
            _ => return Err(FsError::NotSupported),
        }
        Ok(())
    }

    /// `ctl run`: seals input and runs the job to completion on this thread.
    /// A second `run` on a running or finished job is an idempotent no-op
    /// (ADR 0009). Pre-runner validation failures (bad params, oversize
    /// input, quotas) produce a retained failed job, not a write error.
    pub(crate) fn run(&self, principal: &ToolPrincipal, id: &str) -> FsResult<()> {
        let now = self.now();
        let (input, params) = {
            let mut table = self.locked(now)?;
            if !matches!(
                table.get(principal, id)?.state,
                JobState::Allocated | JobState::Receiving
            ) {
                return Ok(());
            }
            let (_, principal_bytes) = table.usage(principal);
            let running = table.running_count(principal);
            let job = table.get_mut(principal, id)?;
            transition(job, JobState::Running, id)?;
            job.started_at = Some(now);
            if let Some(error) = self.pre_run_error(job, principal_bytes, running) {
                self.finalize(job, now, RunOutcome::failure(error, None, Vec::new()));
                return Ok(());
            }
            (job.input.clone(), job.params.clone())
        };
        let outcome = self.inner.runner.run(&input, params.as_ref());
        let finished = self.now();
        let mut table = self.locked(finished)?;
        if let Ok(job) = table.get_mut(principal, id) {
            self.finalize(job, finished, outcome);
        }
        Ok(())
    }

    /// `ctl abort`: terminal jobs are a no-op; pre-run jobs go straight to
    /// `aborted`; a running job is marked and the runner's best-effort
    /// cancellation hook is invoked outside the lock.
    pub(crate) fn abort(&self, principal: &ToolPrincipal, id: &str) -> FsResult<()> {
        let now = self.now();
        let runner = {
            let mut table = self.locked(now)?;
            let job = table.get_mut(principal, id)?;
            match job.state {
                JobState::Done | JobState::Failed | JobState::Aborted => return Ok(()),
                JobState::Allocated | JobState::Receiving => {
                    job.abort_requested = true;
                    self.finalize(job, now, RunOutcome::default());
                    return Ok(());
                }
                JobState::Running => {
                    job.abort_requested = true;
                    Arc::clone(&self.inner.runner)
                }
            }
        };
        if runner.abort_supported() {
            runner.abort(id);
        }
        Ok(())
    }

    /// `ctl close`: deletes a terminal job; anything else is an error.
    pub(crate) fn close(&self, principal: &ToolPrincipal, id: &str) -> FsResult<()> {
        let mut table = self.locked(self.now())?;
        if !table.get(principal, id)?.state.is_terminal() {
            return Err(FsError::Other(format!("job {id} not finished")));
        }
        table.remove(id);
        Ok(())
    }

    /// Rejects an append that would push the table-wide stored-byte total over
    /// [`crate::spec::ToolLimits::max_total_bytes`]. This is the aggregate
    /// memory guardrail: per-principal quotas reset with every fresh dialer
    /// identity, so only a table-wide bound actually limits served memory.
    fn ensure_total_bytes_headroom(&self, total_bytes: u64, incoming: usize) -> FsResult<()> {
        let cap = self.inner.spec.limits.max_total_bytes;
        if total_bytes.saturating_add(incoming as u64) > cap {
            return Err(FsError::Other(format!(
                "quota_exceeded: tool already stores its byte cap ({cap} bytes) across all callers"
            )));
        }
        Ok(())
    }

    fn pre_run_error(
        &self,
        job: &JobRecord,
        principal_bytes: u64,
        running: u64,
    ) -> Option<JobError> {
        let spec = &self.inner.spec;
        if spec.params.required && job.params.is_none() && job.params_raw.is_empty() {
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
        if job.input.len() as u64 > spec.input.max_bytes {
            return Some(JobError::new(
                ErrorKind::InputTooLarge,
                format!("input exceeds maxBytes {}", spec.input.max_bytes),
            ));
        }
        if running >= spec.limits.max_concurrent_per_principal {
            return Some(JobError::new(
                ErrorKind::QuotaExceeded,
                "maxConcurrentPerPrincipal reached",
            ));
        }
        if principal_bytes > spec.limits.max_bytes_per_principal {
            return Some(JobError::new(
                ErrorKind::QuotaExceeded,
                "maxBytesPerPrincipal exceeded",
            ));
        }
        None
    }

    /// Moves a job to its terminal state and records the retained result.
    /// An abort request wins over whatever the runner returned.
    fn finalize(&self, job: &mut JobRecord, now: u64, outcome: RunOutcome) {
        let (state, error) = if job.abort_requested {
            (
                JobState::Aborted,
                Some(JobError::new(ErrorKind::Aborted, "aborted by caller")),
            )
        } else if let Some(error) = outcome.error {
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
        let retain = match state {
            JobState::Done => self.inner.spec.lifecycle.retain_done_ms,
            _ => self.inner.spec.lifecycle.retain_failed_ms,
        };
        job.expires_at = Some(now.saturating_add(retain));
        let retryable = match &error {
            None => self.inner.spec.retryable,
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
}

/// Validates a state change through `wanix_job`'s pure transition relation.
fn transition(job: &mut JobRecord, next: JobState, id: &str) -> FsResult<()> {
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
fn ensure_receiving(job: &mut JobRecord, id: &str) -> FsResult<()> {
    match job.state {
        JobState::Allocated => transition(job, JobState::Receiving, id),
        JobState::Receiving => Ok(()),
        _ => Err(FsError::Other(format!("job {id} input is sealed"))),
    }
}
