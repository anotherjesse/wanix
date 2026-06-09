//! `ToolService`: the shared state behind one ToolFS resource.
//!
//! The service owns the spec, the runner, the injectable clock, and the job
//! table. [`ToolService::open_view`] binds a [`ToolPrincipal`] to a
//! [`ToolFs`] view; every job operation here is principal-scoped. The runner
//! is always invoked with the job-table lock released.

use std::sync::{Arc, Mutex, MutexGuard};

mod lifecycle;

use serde_json::Value;
use wanix_fs::{FsError, FsResult};

use crate::fs::ToolFs;
use crate::jobs::JobTable;
use crate::principal::ToolPrincipal;
use crate::runner::ToolRunner;
use crate::spec::ToolSpec;

/// The injectable time source: Unix-epoch milliseconds.
pub type ToolClock = Box<dyn Fn() -> u64 + Send + Sync>;

/// Shared state of one ToolFS resource; cheap to clone.
#[derive(Clone)]
pub struct ToolService {
    inner: Arc<Inner>,
}

struct Inner {
    spec: ToolSpec,
    params_schema: Option<Vec<u8>>,
    runner: Arc<dyn ToolRunner>,
    clock: ToolClock,
    table: Mutex<JobTable>,
}

impl std::fmt::Debug for ToolService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolService")
            .field("tool", &self.inner.spec.envelope.name)
            .finish()
    }
}

impl ToolService {
    /// A service with no params schema file.
    pub fn new(spec: ToolSpec, runner: Box<dyn ToolRunner>, clock: ToolClock) -> Self {
        Self::with_schema(spec, None, runner, clock)
    }

    /// A service that also serves `params.schema.json`. The spec's
    /// `params.schemaPath` is set to advertise it.
    pub fn with_schema(
        mut spec: ToolSpec,
        params_schema: Option<Value>,
        runner: Box<dyn ToolRunner>,
        clock: ToolClock,
    ) -> Self {
        let params_schema = params_schema.map(|schema| {
            spec.params.schema_path = Some("params.schema.json".to_owned());
            let mut bytes = schema.to_string().into_bytes();
            bytes.push(b'\n');
            bytes
        });
        Self {
            inner: Arc::new(Inner {
                spec,
                params_schema,
                runner: Arc::from(runner),
                clock,
                table: Mutex::new(JobTable::new()),
            }),
        }
    }

    /// Binds `principal` to a filesystem view of this service. The returned
    /// `FileSystem` is principal-blind: the identity is baked in here, at the
    /// attach seam, never read from the caller's payloads.
    #[must_use]
    pub fn open_view(&self, principal: ToolPrincipal) -> ToolFs {
        ToolFs::new(self.clone(), principal)
    }

    /// The service's spec (the parsed form of `spec.json`).
    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        &self.inner.spec
    }

    pub(crate) fn now(&self) -> u64 {
        (self.inner.clock)()
    }

    /// Locks the job table and applies lazy TTL expiry first.
    fn locked(&self, now: u64) -> FsResult<MutexGuard<'_, JobTable>> {
        let mut table = self
            .inner
            .table
            .lock()
            .map_err(|_| FsError::Other("tool job table lock poisoned".to_owned()))?;
        table.expire(now, &self.inner.spec.lifecycle);
        Ok(table)
    }

    pub(crate) fn spec_json(&self) -> Vec<u8> {
        json_line(&self.inner.spec)
    }

    pub(crate) fn schema_json(&self) -> Option<Vec<u8>> {
        self.inner.params_schema.clone()
    }

    pub(crate) fn health_json(&self) -> Vec<u8> {
        b"{\"ok\":true}\n".to_vec()
    }

    pub(crate) fn usage_json(&self, principal: &ToolPrincipal) -> FsResult<Vec<u8>> {
        let (jobs, bytes) = self.locked(self.now())?.usage(principal);
        Ok(json_line(
            &serde_json::json!({ "jobs": jobs, "bytes": bytes }),
        ))
    }

    pub(crate) fn alloc(&self, principal: &ToolPrincipal) -> FsResult<String> {
        let now = self.now();
        self.locked(now)?.alloc(
            principal,
            now,
            self.inner.spec.limits.max_jobs_per_principal,
        )
    }

    pub(crate) fn job_ids(&self, principal: &ToolPrincipal) -> FsResult<Vec<String>> {
        Ok(self.locked(self.now())?.ids_for(principal))
    }

    pub(crate) fn check_job(&self, principal: &ToolPrincipal, id: &str) -> FsResult<()> {
        self.locked(self.now())?.get(principal, id).map(|_| ())
    }

    /// Reads one job field as a byte snapshot. `out`, `err`, and
    /// `result.json` are gated on the job being finished; `status` and the
    /// request fields are readable in every state.
    pub(crate) fn read_field(
        &self,
        principal: &ToolPrincipal,
        id: &str,
        field: JobField,
    ) -> FsResult<Vec<u8>> {
        let table = self.locked(self.now())?;
        let job = table.get(principal, id)?;
        match field {
            JobField::In => Ok(job.input.clone()),
            JobField::Params => Ok(job.params_raw.clone()),
            JobField::Status => Ok(json_line(&job.status())),
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

    /// Current byte length of one job file, for stat/read_dir. Unlike
    /// [`ToolService::read_field`] this never gates on the job being
    /// finished: stat of the fixed job-directory shape must not error.
    pub(crate) fn field_len(
        &self,
        principal: &ToolPrincipal,
        id: &str,
        field: JobField,
    ) -> FsResult<u64> {
        let table = self.locked(self.now())?;
        let job = table.get(principal, id)?;
        Ok(match field {
            JobField::In => job.input.len() as u64,
            JobField::Params => job.params_raw.len() as u64,
            JobField::Out => job.out.len() as u64,
            JobField::Err => job.err.len() as u64,
            JobField::Status => json_line(&job.status()).len() as u64,
            JobField::Result => job
                .result
                .as_ref()
                .map_or(0, |result| json_line(result).len() as u64),
        })
    }
}

/// Which job file a read or reset addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobField {
    In,
    Params,
    Out,
    Err,
    Status,
    Result,
}

fn json_line(value: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    bytes.push(b'\n');
    bytes
}
