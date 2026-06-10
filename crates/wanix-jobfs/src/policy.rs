//! Quotas, retention, and the injectable clock — the policy half of a job
//! device's spec (`docs/toolfs.md` §"Spec Shape"). JSON keys are camelCase so
//! these structs embed directly in a device's `spec.json` shape.

use serde::{Deserialize, Serialize};

/// The injectable time source: Unix-epoch milliseconds.
pub type JobClock = Box<dyn Fn() -> u64 + Send + Sync>;

/// Quotas and the run time budget.
///
/// `run_timeout_ms` feeds the per-run deadline handed to the runner through
/// [`crate::RunContext`]; [`crate::JobCore`] also keeps a finalize-time
/// backstop, so a runner that ignored its deadline still records a `timeout`
/// failure. `0` means no time budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLimits {
    /// Maximum wall-clock run duration in milliseconds (`0` = unlimited).
    pub run_timeout_ms: u64,
    /// Maximum simultaneously running jobs per principal.
    pub max_concurrent_per_principal: u64,
    /// Maximum live (non-expired) jobs per principal.
    pub max_jobs_per_principal: u64,
    /// Maximum stored bytes (`in` + `out` + `err`) per principal.
    pub max_bytes_per_principal: u64,
    /// Maximum live (non-expired) jobs across ALL principals.
    ///
    /// Per-principal quotas alone do not bound a served device's memory: an
    /// open-admission endpoint hands every fresh dialer identity a fresh
    /// quota, so the job table needs an aggregate guardrail.
    #[serde(default = "default_max_total_jobs")]
    pub max_total_jobs: u64,
    /// Maximum stored bytes (inputs, buffered params, outputs, diagnostics)
    /// across ALL principals — the aggregate memory bound, enforced as input
    /// and params bytes arrive and again when a run's outputs are stored.
    #[serde(default = "default_max_total_bytes")]
    pub max_total_bytes: u64,
    /// Maximum stored bytes for one job's primary output (`out`).
    ///
    /// Enforced at finalize: the stored bytes are always clamped to the cap
    /// (the memory bound holds whatever the runner produced), and a run that
    /// would otherwise have succeeded records a `runner_failed` result — a
    /// silently truncated output is worse for a caller than an explicit
    /// failure, and the declared cap is part of the runner's contract.
    #[serde(default = "default_max_out_bytes")]
    pub max_out_bytes: u64,
    /// Maximum stored bytes for one job's diagnostics (`err`); same
    /// clamp-and-fail policy as [`JobLimits::max_out_bytes`].
    #[serde(default = "default_max_err_bytes")]
    pub max_err_bytes: u64,
}

fn default_max_total_jobs() -> u64 {
    1_024
}

fn default_max_total_bytes() -> u64 {
    268_435_456 // 256 MiB
}

fn default_max_out_bytes() -> u64 {
    16_777_216 // 16 MiB: one job may fill a default per-principal byte quota.
}

fn default_max_err_bytes() -> u64 {
    1_048_576 // 1 MiB of diagnostics is plenty for a failed run.
}

impl Default for JobLimits {
    fn default() -> Self {
        Self {
            run_timeout_ms: 5_000,
            max_concurrent_per_principal: 2,
            max_jobs_per_principal: 32,
            max_bytes_per_principal: 16_777_216,
            max_total_jobs: default_max_total_jobs(),
            max_total_bytes: default_max_total_bytes(),
            max_out_bytes: default_max_out_bytes(),
            max_err_bytes: default_max_err_bytes(),
        }
    }
}

/// Job retention lifecycle (all milliseconds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLifecycle {
    /// How long an allocated-but-never-run job lives.
    pub allocated_ttl_ms: u64,
    /// How long a `done` job stays inspectable.
    pub retain_done_ms: u64,
    /// How long a `failed` or `aborted` job stays inspectable.
    pub retain_failed_ms: u64,
}

impl Default for JobLifecycle {
    fn default() -> Self {
        Self {
            allocated_ttl_ms: 300_000,
            retain_done_ms: 600_000,
            retain_failed_ms: 3_600_000,
        }
    }
}

/// Everything [`crate::JobCore`] needs from a device's spec: quotas,
/// retention, and the pre-run validation knobs. The device crate builds this
/// from its own spec shape (e.g. `wanix-tool` from `ToolSpec`).
#[derive(Debug, Clone)]
pub struct JobPolicy {
    /// Quotas and the run time budget.
    pub limits: JobLimits,
    /// Job retention lifecycle.
    pub lifecycle: JobLifecycle,
    /// Whether `params.json` must be written before `ctl run`.
    pub params_required: bool,
    /// Maximum accepted input size in bytes (`input_too_large` past it).
    pub max_input_bytes: u64,
    /// The device's `retryable` verdict for successful runs.
    pub retryable: bool,
}
