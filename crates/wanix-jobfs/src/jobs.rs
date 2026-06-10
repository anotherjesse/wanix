//! [`JobCore`]: the shared job-table state behind one job device.
//!
//! The core owns the policy, the runner, the injectable clock, and the job
//! table; the device crate binds a [`JobPrincipal`] per view and routes its
//! filesystem surface here. Every operation is principal-scoped. The runner
//! is always invoked with the job-table lock released.

use std::sync::{Arc, Mutex, MutexGuard};

use wanix_fs::{FsError, FsResult, LineBuffer};

use crate::policy::{JobClock, JobPolicy};
use crate::principal::JobPrincipal;
use crate::runner::JobRunner;
use crate::table::JobTable;

/// Shared job-protocol state of one device; cheap to clone.
#[derive(Clone)]
pub struct JobCore {
    inner: Arc<Inner>,
}

struct Inner {
    policy: JobPolicy,
    runner: Arc<dyn JobRunner>,
    clock: JobClock,
    table: Mutex<JobTable>,
}

impl std::fmt::Debug for JobCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobCore").finish_non_exhaustive()
    }
}

impl JobCore {
    /// A core enforcing `policy`, running jobs through `runner`, reading time
    /// (Unix-epoch milliseconds) from `clock`.
    #[must_use]
    pub fn new(policy: JobPolicy, runner: Arc<dyn JobRunner>, clock: JobClock) -> Self {
        Self {
            inner: Arc::new(Inner {
                policy,
                runner,
                clock,
                table: Mutex::new(JobTable::new()),
            }),
        }
    }

    pub(crate) fn policy(&self) -> &JobPolicy {
        &self.inner.policy
    }

    pub(crate) fn runner(&self) -> &Arc<dyn JobRunner> {
        &self.inner.runner
    }

    pub(crate) fn now(&self) -> u64 {
        (self.inner.clock)()
    }

    /// Locks the job table and applies lazy TTL expiry first.
    pub(crate) fn locked(&self, now: u64) -> FsResult<MutexGuard<'_, JobTable>> {
        let mut table = self
            .inner
            .table
            .lock()
            .map_err(|_| FsError::Other("job table lock poisoned".to_owned()))?;
        table.expire(now, &self.inner.policy.lifecycle);
        Ok(table)
    }

    /// Allocates a job for `principal` and returns its opaque id.
    ///
    /// # Errors
    ///
    /// Returns a `quota_exceeded` error when the per-principal or table-wide
    /// job cap is reached.
    pub fn alloc(&self, principal: &JobPrincipal) -> FsResult<String> {
        let now = self.now();
        self.locked(now)?
            .alloc(principal, now, &self.inner.policy.limits)
    }

    /// This principal's live job ids.
    ///
    /// # Errors
    ///
    /// Returns an error only when the table lock is poisoned.
    pub fn job_ids(&self, principal: &JobPrincipal) -> FsResult<Vec<String>> {
        Ok(self.locked(self.now())?.ids_for(principal))
    }

    /// Confirms `id` is a live job of `principal` (`NotFound` otherwise).
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign or missing id.
    pub fn check_job(&self, principal: &JobPrincipal, id: &str) -> FsResult<()> {
        self.locked(self.now())?.get(principal, id).map(|_| ())
    }

    /// The `usage` file body: this principal's live job count and stored
    /// bytes, as one JSON line.
    ///
    /// # Errors
    ///
    /// Returns an error only when the table lock is poisoned.
    pub fn usage_json(&self, principal: &JobPrincipal) -> FsResult<Vec<u8>> {
        let (jobs, bytes) = self.locked(self.now())?.usage(principal);
        Ok(json_line(
            &serde_json::json!({ "jobs": jobs, "bytes": bytes }),
        ))
    }

    /// Reads one job field as a byte snapshot. `out`, `err`, and
    /// `result.json` are gated on the job being finished; `status` and the
    /// request fields are readable in every state. `events` is a stream, not
    /// a snapshot — open it through [`JobCore::events_handle`].
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign or missing id and an `Other`
    /// not-finished error for result fields of a live job.
    pub fn read_field(
        &self,
        principal: &JobPrincipal,
        id: &str,
        field: JobField,
    ) -> FsResult<Vec<u8>> {
        let table = self.locked(self.now())?;
        let job = table.get(principal, id)?;
        match field {
            JobField::In => Ok(job.input.clone()),
            JobField::Params => Ok(job.params_raw.clone()),
            JobField::Status => Ok(json_line(&job.status())),
            JobField::Events => Err(FsError::NotSupported),
            JobField::Out | JobField::Err | JobField::Result => {
                if !job.state.is_terminal() {
                    return Err(FsError::Other(format!("job {id} not finished")));
                }
                match field {
                    JobField::Out => Ok(job.out.clone()),
                    JobField::Err => Ok(job.err.clone()),
                    _ => {
                        let result = job.result.as_ref().ok_or_else(|| {
                            FsError::Other(format!("job {id} has no result record"))
                        })?;
                        Ok(json_line(result))
                    }
                }
            }
        }
    }

    /// The job's `events` buffer, for a stream read held open across table
    /// mutations: the `Arc` is cloned under the lock, the blocking reads
    /// happen outside it.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign or missing id.
    pub fn events_handle(&self, principal: &JobPrincipal, id: &str) -> FsResult<Arc<LineBuffer>> {
        let table = self.locked(self.now())?;
        Ok(Arc::clone(&table.get(principal, id)?.events))
    }

    /// Current byte length of one job file, for stat/read_dir. Unlike
    /// [`JobCore::read_field`] this never gates on the job being finished:
    /// stat of the fixed job-directory shape must not error. Streams
    /// (`events`) report 0.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for a foreign or missing id.
    pub fn field_len(&self, principal: &JobPrincipal, id: &str, field: JobField) -> FsResult<u64> {
        let table = self.locked(self.now())?;
        let job = table.get(principal, id)?;
        Ok(match field {
            JobField::In => job.input.len() as u64,
            JobField::Params => job.params_raw.len() as u64,
            JobField::Out => job.out.len() as u64,
            JobField::Err => job.err.len() as u64,
            JobField::Status => json_line(&job.status()).len() as u64,
            JobField::Events => 0,
            JobField::Result => job
                .result
                .as_ref()
                .map_or(0, |result| json_line(result).len() as u64),
        })
    }
}

/// Which job file a read or reset addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobField {
    /// Request bytes (`in`).
    In,
    /// Structured parameters (`params.json`).
    Params,
    /// Primary output (`out`).
    Out,
    /// Diagnostics (`err`).
    Err,
    /// Live state snapshot (`status`).
    Status,
    /// Retained final summary (`result.json`).
    Result,
    /// Progress stream (`events`); a never-EOF read while the job lives.
    Events,
}

pub(crate) fn json_line(value: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    bytes.push(b'\n');
    bytes
}
