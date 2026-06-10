//! `ToolService`: the spec surface of one ToolFS resource over the shared
//! job machinery.
//!
//! The service owns the spec and the params schema; the job table, quotas,
//! lifecycle, and runner dispatch live in [`wanix_jobfs::JobCore`].
//! [`ToolService::open_view`] binds a [`JobPrincipal`] to a [`ToolFs`] view;
//! every job operation is principal-scoped, and the runner is always invoked
//! with the job-table lock released.

use std::sync::Arc;

use serde_json::Value;
use wanix_fs::{FsError, FsResult};
use wanix_jobfs::{JobClock, JobCore, JobPolicy, JobPrincipal, JobRunner};

use crate::fs::ToolFs;
use crate::spec::{ToolSpec, ToolVisibility};

/// Shared state of one ToolFS resource; cheap to clone.
#[derive(Clone)]
pub struct ToolService {
    inner: Arc<Inner>,
}

struct Inner {
    spec: ToolSpec,
    params_schema: Option<Vec<u8>>,
    core: JobCore,
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
    ///
    /// # Errors
    ///
    /// Rejects a spec whose `visibility` is not `private`: v0 implements
    /// private job views only, and advertising an unimplemented mode would be
    /// a dishonest spec.
    pub fn new(spec: ToolSpec, runner: Box<dyn JobRunner>, clock: JobClock) -> FsResult<Self> {
        Self::with_schema(spec, None, runner, clock)
    }

    /// A service that also serves `params.schema.json`. The spec's
    /// `params.schemaPath` is set to advertise it (advertisement only: params
    /// are validated for JSON well-formedness, not schema conformance).
    ///
    /// # Errors
    ///
    /// Rejects a non-`private` `visibility` (see [`ToolService::new`]).
    pub fn with_schema(
        mut spec: ToolSpec,
        params_schema: Option<Value>,
        runner: Box<dyn JobRunner>,
        clock: JobClock,
    ) -> FsResult<Self> {
        if spec.visibility != ToolVisibility::Private {
            return Err(FsError::Other(format!(
                "tool spec visibility {:?} is not implemented: v0 serves private job views only",
                spec.visibility.as_str()
            )));
        }
        let params_schema = params_schema.map(|schema| {
            spec.params.schema_path = Some("params.schema.json".to_owned());
            let mut bytes = schema.to_string().into_bytes();
            bytes.push(b'\n');
            bytes
        });
        let policy = JobPolicy {
            limits: spec.limits.clone(),
            lifecycle: spec.lifecycle.clone(),
            params_required: spec.params.required,
            max_input_bytes: spec.input.max_bytes,
            retryable: spec.retryable,
        };
        Ok(Self {
            inner: Arc::new(Inner {
                spec,
                params_schema,
                core: JobCore::new(policy, Arc::from(runner), clock),
            }),
        })
    }

    /// Binds `principal` to a filesystem view of this service. The returned
    /// `FileSystem` is principal-blind: the identity is baked in here, at the
    /// attach seam, never read from the caller's payloads.
    #[must_use]
    pub fn open_view(&self, principal: JobPrincipal) -> ToolFs {
        ToolFs::new(self.clone(), principal)
    }

    /// The service's spec (the parsed form of `spec.json`).
    #[must_use]
    pub fn spec(&self) -> &ToolSpec {
        &self.inner.spec
    }

    /// The shared job machinery behind this tool's job directories.
    pub(crate) fn core(&self) -> &JobCore {
        &self.inner.core
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
}

fn json_line(value: &impl serde::Serialize) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    bytes.push(b'\n');
    bytes
}
