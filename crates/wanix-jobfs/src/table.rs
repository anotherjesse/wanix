//! The job table: per-job records, opaque ids, lazy TTL expiry, and the
//! per-principal accounting behind quotas and the `usage` file.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use wanix_fs::{FsError, FsResult, LineBuffer};
use wanix_job::{JobState, JobStatus};

use crate::policy::{JobLifecycle, JobLimits};
use crate::principal::JobPrincipal;

/// Ceiling on one job's buffered `events` backlog. Progress is a bounded,
/// lossy stream (drop-oldest, like every subscription buffer): a job with no
/// `events` reader must not let a chatty runner grow host memory.
const EVENTS_BUFFER_BYTES: usize = 64 * 1024;

/// One job's full record: lifecycle, timestamps, stored bytes, and result.
#[derive(Debug)]
pub(crate) struct JobRecord {
    pub(crate) principal: JobPrincipal,
    pub(crate) state: JobState,
    pub(crate) created_at: u64,
    pub(crate) started_at: Option<u64>,
    pub(crate) finished_at: Option<u64>,
    /// Retention deadline; set only once the job is terminal.
    pub(crate) expires_at: Option<u64>,
    pub(crate) input: Vec<u8>,
    /// Raw bytes written to `params.json` so far.
    pub(crate) params_raw: Vec<u8>,
    /// The committed params value, once `params_raw` parses as full JSON.
    pub(crate) params: Option<serde_json::Value>,
    pub(crate) out: Vec<u8>,
    pub(crate) err: Vec<u8>,
    pub(crate) result: Option<wanix_job::JobResult>,
    /// `ctl abort` sets this; the running [`crate::RunContext`] shares it, so
    /// the abort is correlatable to the exact run.
    pub(crate) abort: Arc<AtomicBool>,
    /// The job's `events` progress stream: fed by the runner through its
    /// [`crate::RunContext`], drained by `events` readers outside the table
    /// lock, closed when the job finishes or is dropped.
    pub(crate) events: Arc<LineBuffer>,
}

impl JobRecord {
    fn new(principal: JobPrincipal, now: u64) -> Self {
        Self {
            principal,
            state: JobState::Allocated,
            created_at: now,
            started_at: None,
            finished_at: None,
            expires_at: None,
            input: Vec::new(),
            params_raw: Vec::new(),
            params: None,
            out: Vec::new(),
            err: Vec::new(),
            result: None,
            abort: Arc::new(AtomicBool::new(false)),
            events: Arc::new(LineBuffer::bounded(EVENTS_BUFFER_BYTES)),
        }
    }

    /// The live `status` snapshot for this record.
    pub(crate) fn status(&self) -> JobStatus {
        JobStatus {
            state: self.state,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            expires_at: self.expires_at,
            input_bytes: self.input.len() as u64,
            output_bytes: self.out.len() as u64,
        }
    }

    /// Bytes this job holds against the per-principal byte quota.
    pub(crate) fn stored_bytes(&self) -> u64 {
        (self.input.len() + self.out.len() + self.err.len()) as u64
    }
}

/// The shared job table, kept behind the core mutex.
#[derive(Debug)]
pub(crate) struct JobTable {
    seed: u64,
    counter: u64,
    jobs: BTreeMap<String, JobRecord>,
}

impl JobTable {
    pub(crate) fn new() -> Self {
        // Seed entropy for opaque ids only; never used as a job timestamp.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
            .unwrap_or(0);
        Self {
            seed,
            counter: 0,
            jobs: BTreeMap::new(),
        }
    }

    /// Drops expired jobs: allocated/receiving past the allocated TTL and
    /// terminal jobs past their retention deadline. Running jobs never expire.
    /// A dropped job's `events` buffer is closed so a blocked reader observes
    /// EOF instead of parking on a record that no longer exists.
    pub(crate) fn expire(&mut self, now: u64, lifecycle: &JobLifecycle) {
        let allocated_ttl = lifecycle.allocated_ttl_ms;
        self.jobs.retain(|_, job| {
            let keep = match job.state {
                JobState::Running => true,
                JobState::Allocated | JobState::Receiving => {
                    now < job.created_at.saturating_add(allocated_ttl)
                }
                JobState::Done | JobState::Failed | JobState::Aborted => {
                    job.expires_at.is_none_or(|deadline| now < deadline)
                }
            };
            if !keep {
                job.events.close();
            }
            keep
        });
    }

    /// Allocates a job for `principal`, enforcing the per-principal job cap
    /// and the table-wide [`JobLimits::max_total_jobs`] cap (per-principal
    /// quotas alone are non-limiting when every dialer can mint a fresh
    /// identity).
    pub(crate) fn alloc(
        &mut self,
        principal: &JobPrincipal,
        now: u64,
        limits: &JobLimits,
    ) -> FsResult<String> {
        if self.jobs.len() as u64 >= limits.max_total_jobs {
            return Err(FsError::Other(format!(
                "quota_exceeded: device already holds {} live jobs across all callers",
                limits.max_total_jobs
            )));
        }
        if self.job_count(principal) >= limits.max_jobs_per_principal {
            return Err(FsError::Other(format!(
                "quota_exceeded: principal already holds {} jobs",
                limits.max_jobs_per_principal
            )));
        }
        let id = self.next_id(principal);
        self.jobs
            .insert(id.clone(), JobRecord::new(principal.clone(), now));
        Ok(id)
    }

    /// Total stored bytes across ALL principals, counting buffered params too:
    /// every retained byte counts against the aggregate memory bound, whatever
    /// the per-principal accounting reports.
    pub(crate) fn total_bytes(&self) -> u64 {
        self.jobs
            .values()
            .map(|job| job.stored_bytes() + job.params_raw.len() as u64)
            .sum()
    }

    /// Looks up `id` for `principal`. A foreign or missing job id is
    /// `NotFound` — never `PermissionDenied` (ADR 0009 §privacy).
    pub(crate) fn get(&self, principal: &JobPrincipal, id: &str) -> FsResult<&JobRecord> {
        self.jobs
            .get(id)
            .filter(|job| job.principal == *principal)
            .ok_or(FsError::NotFound)
    }

    /// Mutable [`JobTable::get`].
    pub(crate) fn get_mut(
        &mut self,
        principal: &JobPrincipal,
        id: &str,
    ) -> FsResult<&mut JobRecord> {
        self.jobs
            .get_mut(id)
            .filter(|job| job.principal == *principal)
            .ok_or(FsError::NotFound)
    }

    /// Deletes `id`, releasing any blocked `events` reader with EOF.
    pub(crate) fn remove(&mut self, id: &str) {
        if let Some(job) = self.jobs.remove(id) {
            job.events.close();
        }
    }

    /// This principal's job ids, in stable (BTreeMap) order.
    pub(crate) fn ids_for(&self, principal: &JobPrincipal) -> Vec<String> {
        self.jobs
            .iter()
            .filter(|(_, job)| job.principal == *principal)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Live job count and stored byte total for `principal` (the `usage`
    /// numbers and the quota inputs).
    pub(crate) fn usage(&self, principal: &JobPrincipal) -> (u64, u64) {
        let mut jobs = 0u64;
        let mut bytes = 0u64;
        for job in self.jobs.values() {
            if job.principal == *principal {
                jobs += 1;
                bytes += job.stored_bytes();
            }
        }
        (jobs, bytes)
    }

    /// How many of this principal's jobs are currently running.
    pub(crate) fn running_count(&self, principal: &JobPrincipal) -> u64 {
        self.jobs
            .values()
            .filter(|job| job.principal == *principal && job.state == JobState::Running)
            .count() as u64
    }

    fn job_count(&self, principal: &JobPrincipal) -> u64 {
        self.usage(principal).0
    }

    /// An opaque id: hashed from seed, allocation counter, and principal so
    /// ids are not guessable-sequential across principals.
    fn next_id(&mut self, principal: &JobPrincipal) -> String {
        loop {
            self.counter += 1;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.seed.hash(&mut hasher);
            self.counter.hash(&mut hasher);
            principal.hash(&mut hasher);
            let id = format!("j{:016x}", hasher.finish());
            if !self.jobs.contains_key(&id) {
                return id;
            }
        }
    }
}
