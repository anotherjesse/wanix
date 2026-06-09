//! The `spec.json` contract shapes (`docs/toolfs.md` §"Spec Shape").
//!
//! [`ToolSpec`] is the mounted source of truth for one ToolFS resource. It
//! opens with the shared `wanix.resource` envelope from `wanix-job` and adds
//! the tool-specific sections: input, params, outputs, limits, lifecycle,
//! visibility, and declared effects. JSON keys are camelCase.

use serde::{Deserialize, Serialize};
use wanix_job::ResourceEnvelope;

/// The machine-readable contract of one ToolFS resource (`spec.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    /// The shared `"wanix.resource": "v0"` envelope, `kind` `"tool"`.
    #[serde(flatten)]
    pub envelope: ResourceEnvelope,
    /// What the job `in` file accepts.
    pub input: ToolInput,
    /// How `params.json` is described.
    pub params: ToolParams,
    /// Where the job's outputs land.
    pub outputs: ToolOutputs,
    /// Quotas and the run time budget.
    pub limits: ToolLimits,
    /// Job retention lifecycle.
    pub lifecycle: ToolLifecycle,
    /// Job visibility across principals (v0 always behaves as `private`).
    pub visibility: ToolVisibility,
    /// Declared effects (ADR 0009 §Discovery) — a trust statement by the
    /// device author, not something the platform can enforce.
    pub side_effects: ToolSideEffects,
    /// Whether re-running a successful call (as a new job) is sensible.
    pub retryable: bool,
}

impl ToolSpec {
    /// A `v0` tool spec with the `docs/toolfs.md` sketch defaults.
    pub fn v0(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            envelope: ResourceEnvelope::v0("tool", name, description),
            input: ToolInput::default(),
            params: ToolParams::default(),
            outputs: ToolOutputs::default(),
            limits: ToolLimits::default(),
            lifecycle: ToolLifecycle::default(),
            visibility: ToolVisibility::Private,
            side_effects: ToolSideEffects::None,
            retryable: true,
        }
    }
}

/// What the job `in` file accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInput {
    /// Input mode; v0 supports `"bytes"` only.
    pub mode: String,
    /// Advisory content types the tool understands.
    pub content_types: Vec<String>,
    /// Maximum accepted input size in bytes (`input_too_large` past it).
    pub max_bytes: u64,
}

impl Default for ToolInput {
    fn default() -> Self {
        Self {
            mode: "bytes".to_owned(),
            content_types: vec!["text/plain; charset=utf-8".to_owned()],
            max_bytes: 1_048_576,
        }
    }
}

/// How `params.json` is described.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolParams {
    /// Path of the advertised JSON schema file, when one is configured.
    /// The schema is advertisement: v0 validates JSON well-formedness only.
    pub schema_path: Option<String>,
    /// Whether `params.json` must be written before `ctl run`.
    pub required: bool,
}

/// Where the job's outputs land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputs {
    /// The primary output stream.
    pub primary: ToolOutput,
    /// The diagnostics stream.
    pub diagnostics: ToolOutput,
}

impl Default for ToolOutputs {
    fn default() -> Self {
        Self {
            primary: ToolOutput {
                path: "out".to_owned(),
                content_type: Some("text/plain; charset=utf-8".to_owned()),
            },
            diagnostics: ToolOutput {
                path: "err".to_owned(),
                content_type: None,
            },
        }
    }
}

/// One output stream description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutput {
    /// File name under the job directory.
    pub path: String,
    /// Advisory content type, when declared.
    pub content_type: Option<String>,
}

/// Quotas and the run time budget.
///
/// `run_timeout_ms` is declared contract; v0's in-process runners complete
/// synchronously and enforcement belongs to the process runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLimits {
    /// Maximum wall-clock run duration in milliseconds.
    pub run_timeout_ms: u64,
    /// Maximum simultaneously running jobs per principal.
    pub max_concurrent_per_principal: u64,
    /// Maximum live (non-expired) jobs per principal.
    pub max_jobs_per_principal: u64,
    /// Maximum stored bytes (`in` + `out` + `err`) per principal.
    pub max_bytes_per_principal: u64,
}

impl Default for ToolLimits {
    fn default() -> Self {
        Self {
            run_timeout_ms: 5_000,
            max_concurrent_per_principal: 2,
            max_jobs_per_principal: 32,
            max_bytes_per_principal: 16_777_216,
        }
    }
}

/// Job retention lifecycle (all milliseconds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolLifecycle {
    /// How long an allocated-but-never-run job lives.
    pub allocated_ttl_ms: u64,
    /// How long a `done` job stays inspectable.
    pub retain_done_ms: u64,
    /// How long a `failed` or `aborted` job stays inspectable.
    pub retain_failed_ms: u64,
}

impl Default for ToolLifecycle {
    fn default() -> Self {
        Self {
            allocated_ttl_ms: 300_000,
            retain_done_ms: 600_000,
            retain_failed_ms: 3_600_000,
        }
    }
}

/// Job visibility across principals (`docs/toolfs.md` §"Privacy").
///
/// v0 implements `private` semantics for every mode; the field is contract
/// advertisement for the modes to come.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolVisibility {
    /// Callers see only their own jobs.
    Private,
    /// All authorized callers share one job set.
    Shared,
    /// Callers see their own jobs; the operator sees all.
    Operator,
    /// Demo-only: no job privacy boundary.
    Public,
}

/// Declared effects of one run (ADR 0009 §Discovery).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolSideEffects {
    /// The run has no externally visible effects.
    None,
    /// Effects exist but repeating the run is safe.
    Idempotent,
    /// Effects must not be repeated; the job id is the dedup key.
    AtMostOnce,
}
