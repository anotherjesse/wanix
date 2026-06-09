//! ToolFS — the first job-protocol device ([ADR 0009]).
//!
//! A ToolFS exposes one host-approved operation family as files
//! (`docs/toolfs.md`): the host fixes the operation and its policy, a caller
//! supplies only input bytes and validated parameters through the mounted
//! filesystem. The visible shape is the job-protocol grammar:
//!
//! ```text
//! spec.json  params.schema.json  health  usage  new
//! jobs/<id>/{in, params.json, ctl, out, err, status, result.json}
//! ```
//!
//! - [`ToolService`] owns the shared job table; [`ToolService::open_view`]
//!   returns a principal-scoped [`ToolFs`] — the `FileSystem` itself stays
//!   principal-blind, and the acting [`ToolPrincipal`] comes from the
//!   transport/attach layer, never from a payload field.
//! - [`ToolRunner`] is the synchronous host-policy seam behind the device;
//!   v0 ships deterministic in-process runners in [`runners`] (the process
//!   runner lives outside this crate per `docs/toolfs.md` §"Runner Boundary").
//! - Job vocabulary (states, the error taxonomy, `status`/`result.json`
//!   shapes, the `wanix.resource` spec envelope) comes from `wanix-job`;
//!   this crate adds the machinery: the job table, lifecycle/TTLs, quotas,
//!   and the filesystem surface.
//!
//! Timestamps are Unix-epoch milliseconds supplied by the injectable service
//! clock; `wanix-job` never reads a clock.
//!
//! [ADR 0009]: ../../../docs/adrs/0009-job-protocol.md

/// Short crate purpose string for workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix tool device filesystem (job protocol)";

mod files;
mod fs;
mod jobs;
mod principal;
mod runner;
pub mod runners;
mod service;
mod spec;

pub use fs::ToolFs;
pub use principal::ToolPrincipal;
pub use runner::{RunOutcome, ToolRunner};
pub use service::{ToolClock, ToolService};
pub use spec::{
    ToolInput, ToolLifecycle, ToolLimits, ToolOutput, ToolOutputs, ToolParams, ToolSideEffects,
    ToolSpec, ToolVisibility,
};

#[cfg(test)]
mod tests;
