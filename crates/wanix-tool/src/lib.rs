//! ToolFS — the first job-protocol device ([ADR 0009]).
//!
//! A ToolFS exposes one host-approved operation family as files
//! (`docs/toolfs.md`): the host fixes the operation and its policy, a caller
//! supplies only input bytes and validated parameters through the mounted
//! filesystem. The visible shape is the job-protocol grammar:
//!
//! ```text
//! spec.json  params.schema.json  health  usage  new
//! jobs/<id>/{in, params.json, ctl, out, err, status, result.json, events}
//! ```
//!
//! - [`ToolService`] owns the spec surface and wraps the shared job machinery
//!   ([`wanix_jobfs::JobCore`]); [`ToolService::open_view`] returns a
//!   principal-scoped [`ToolFs`] — the `FileSystem` itself stays
//!   principal-blind, and the acting [`JobPrincipal`] comes from the
//!   transport/attach layer, never from a payload field.
//! - [`JobRunner`] (re-exported here as `ToolRunner`) is the synchronous
//!   host-policy seam behind the device; v0 ships deterministic in-process
//!   runners in [`runners`] (the process runner lives outside this crate per
//!   `docs/toolfs.md` §"Runner Boundary").
//! - Job vocabulary (states, the error taxonomy, `status`/`result.json`
//!   shapes, the `wanix.resource` spec envelope) comes from `wanix-job`; the
//!   machinery (job table, lifecycle/TTLs, quotas, runner seam, job-dir
//!   `File` impls) comes from `wanix-jobfs`; this crate adds the tool spec
//!   and the filesystem surface.
//!
//! Timestamps are Unix-epoch milliseconds supplied by the injectable service
//! clock; `wanix-job` never reads a clock.
//!
//! [ADR 0009]: ../../../docs/adrs/0009-job-protocol.md

/// Short crate purpose string for workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix tool device filesystem (job protocol)";

mod fs;
pub mod runners;
mod service;
mod spec;

pub use fs::ToolFs;
pub use service::ToolService;
pub use spec::{
    ToolInput, ToolOutput, ToolOutputs, ToolParams, ToolSideEffects, ToolSpec, ToolVisibility,
};
pub use wanix_jobfs::{JobPrincipal, JobRunner, RunContext, RunOutcome};

/// Deprecated for removal: import [`wanix_jobfs::JobClock`] instead.
pub use wanix_jobfs::JobClock as ToolClock;
/// Deprecated for removal: import [`wanix_jobfs::JobLifecycle`] instead.
pub use wanix_jobfs::JobLifecycle as ToolLifecycle;
/// Deprecated for removal: import [`wanix_jobfs::JobLimits`] instead.
pub use wanix_jobfs::JobLimits as ToolLimits;
/// Deprecated for removal: import [`wanix_jobfs::JobPrincipal`] (re-exported
/// here as [`JobPrincipal`]) instead.
pub use wanix_jobfs::JobPrincipal as ToolPrincipal;
/// Deprecated for removal: import [`wanix_jobfs::JobRunner`] instead.
pub use wanix_jobfs::JobRunner as ToolRunner;

#[cfg(test)]
mod tests;
