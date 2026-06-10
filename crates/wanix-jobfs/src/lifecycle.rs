//! The effectful half of [`JobCore`]: input/params mutation and the `ctl`
//! verbs (`run`, `abort`, `close`), with job-state changes validated through
//! `wanix_job`'s pure transition relation. The runner is always invoked with
//! the job-table lock released.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;
use wanix_fs::{FsError, FsResult};
use wanix_job::JobState;

use crate::finalize::{ensure_receiving, transition};
use crate::jobs::{JobCore, JobField};
use crate::principal::JobPrincipal;
use crate::runner::{RunContext, RunOutcome};

impl JobCore {
    /// Appends bytes to a job's `in`. Sealed once `ctl run` is accepted.
    /// Growth is bounded by the table-wide byte cap as the bytes arrive.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign id, a sealed error after run, and a
    /// `quota_exceeded` error past the aggregate byte cap.
    pub fn append_input(
        &self,
        principal: &JobPrincipal,
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
    ///
    /// # Errors
    ///
    /// Same as [`JobCore::append_input`], plus an invalid-JSON error.
    pub fn append_params(
        &self,
        principal: &JobPrincipal,
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
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign id, a sealed error after run, and
    /// `NotSupported` for non-request fields.
    pub fn reset_field(&self, principal: &JobPrincipal, id: &str, field: JobField) -> FsResult<()> {
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
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign id; runner failures land in the
    /// retained result, never here.
    pub fn run(&self, principal: &JobPrincipal, id: &str) -> FsResult<()> {
        let now = self.now();
        let (input, params, context) = {
            let mut table = self.locked(now)?;
            if !matches!(
                table.get(principal, id)?.state,
                JobState::Allocated | JobState::Receiving
            ) {
                return Ok(());
            }
            let (_, principal_bytes) = table.usage(principal);
            let running = table.running_count(principal);
            let headroom = self.output_headroom(table.total_bytes());
            let job = table.get_mut(principal, id)?;
            transition(job, JobState::Running, id)?;
            job.started_at = Some(now);
            if let Some(error) = self.pre_run_error(job, principal_bytes, running) {
                self.finalize(
                    job,
                    now,
                    RunOutcome::failure(error, None, Vec::new()),
                    headroom,
                );
                return Ok(());
            }
            let context = RunContext::new(
                id.to_owned(),
                self.deadline_after(now),
                Arc::clone(&job.abort),
                Arc::clone(&job.events),
            );
            (job.input.clone(), job.params.clone(), context)
        };
        let outcome = self.runner().run(&input, params.as_ref(), &context);
        let finished = self.now();
        let mut table = self.locked(finished)?;
        let headroom = self.output_headroom(table.total_bytes());
        if let Ok(job) = table.get_mut(principal, id) {
            self.finalize(job, finished, outcome, headroom);
        }
        Ok(())
    }

    /// `ctl abort`: terminal jobs are a no-op; pre-run jobs go straight to
    /// `aborted`; a running job has its shared abort flag set (observable
    /// through the run's [`RunContext`]) and the runner's best-effort
    /// cancellation hook is invoked outside the lock.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign or missing id.
    pub fn abort(&self, principal: &JobPrincipal, id: &str) -> FsResult<()> {
        let now = self.now();
        let runner = {
            let mut table = self.locked(now)?;
            let job = table.get_mut(principal, id)?;
            match job.state {
                JobState::Done | JobState::Failed | JobState::Aborted => return Ok(()),
                JobState::Allocated | JobState::Receiving => {
                    job.abort.store(true, Ordering::SeqCst);
                    self.finalize(job, now, RunOutcome::default(), 0);
                    return Ok(());
                }
                JobState::Running => {
                    job.abort.store(true, Ordering::SeqCst);
                    Arc::clone(self.runner())
                }
            }
        };
        if runner.abort_supported() {
            runner.abort(id);
        }
        Ok(())
    }

    /// `ctl close`: deletes a terminal job; anything else is an error.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign id and a not-finished error for a
    /// live one.
    pub fn close(&self, principal: &JobPrincipal, id: &str) -> FsResult<()> {
        let mut table = self.locked(self.now())?;
        if !table.get(principal, id)?.state.is_terminal() {
            return Err(FsError::Other(format!("job {id} not finished")));
        }
        table.remove(id);
        Ok(())
    }

    /// Rejects an append that would push the table-wide stored-byte total over
    /// [`crate::JobLimits::max_total_bytes`]. This is the aggregate memory
    /// guardrail: per-principal quotas reset with every fresh dialer identity,
    /// so only a table-wide bound actually limits served memory.
    fn ensure_total_bytes_headroom(&self, total_bytes: u64, incoming: usize) -> FsResult<()> {
        let cap = self.policy().limits.max_total_bytes;
        if total_bytes.saturating_add(incoming as u64) > cap {
            return Err(FsError::Other(format!(
                "quota_exceeded: device already stores its byte cap ({cap} bytes) across all callers"
            )));
        }
        Ok(())
    }

    /// Remaining aggregate-byte headroom for storing a finished run's
    /// outputs, given the table's current stored total.
    fn output_headroom(&self, total_bytes: u64) -> u64 {
        self.policy()
            .limits
            .max_total_bytes
            .saturating_sub(total_bytes)
    }
}
