//! Job-protocol device machinery ([ADR 0009]).
//!
//! `wanix-job` pins the *vocabulary* every job device speaks (states, the
//! error taxonomy, report shapes). This crate is the shared *machinery* that
//! turns the vocabulary into a working device, so the second adopter
//! (`#agent`, `#cpu`, the process runner) reuses one implementation instead
//! of copying ToolFS:
//!
//! - [`JobPrincipal`] — the transport-derived identity a mounted view acts as.
//! - [`JobLimits`]/[`JobLifecycle`]/[`JobPolicy`]/[`JobClock`] — quotas,
//!   retention, and the injectable time source.
//! - [`JobRunner`]/[`RunOutcome`]/[`RunContext`] — the synchronous host-policy
//!   seam behind a device, with per-run job id, deadline, abort flag, and
//!   progress sink.
//! - [`JobCore`] — the principal-scoped job table plus the lifecycle driver:
//!   allocation, input/params staging, `run`/`abort`/`close`, TTL expiry,
//!   quota enforcement, output caps, and the timeout backstop.
//! - The job-directory `File` implementations ([`NewJobFile`], [`AppendFile`],
//!   [`CtlFile`], [`EventsFile`], [`BytesFile`]) a device's `FileSystem`
//!   hands out.
//!
//! What stays per-device: the `spec.json` shape, the path layout, and the
//! `FileSystem` impl itself (`wanix-tool` is the first consumer). Timestamps
//! are Unix-epoch milliseconds supplied by the injectable clock; `wanix-job`
//! never reads a clock.
//!
//! [ADR 0009]: ../../../docs/adrs/0009-job-protocol.md

/// Short crate purpose string for workspace smoke tests.
pub const CRATE_PURPOSE: &str = "job-protocol device machinery (job table, lifecycle, files)";

mod files;
mod finalize;
mod jobs;
mod lifecycle;
mod policy;
mod principal;
mod runner;
mod table;

pub use files::{
    AppendFile, BytesFile, CtlFile, EventsFile, NewJobFile, directory_metadata, file_metadata,
    modes, require_read_only,
};
pub use jobs::{JobCore, JobField};
pub use policy::{JobClock, JobLifecycle, JobLimits, JobPolicy};
pub use principal::JobPrincipal;
pub use runner::{JobRunner, RunContext, RunOutcome};

#[cfg(test)]
mod tests {
    use super::CRATE_PURPOSE;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }
}
