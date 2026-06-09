//! Job-protocol vocabulary for [ADR 0009] — the reified-call workspace
//! convention.
//!
//! This crate is **vocabulary, not machinery**: the shared value types every
//! job-protocol device speaks, with no I/O, no clock (timestamps are
//! caller-supplied Unix-epoch milliseconds), no filesystem coupling, and no
//! state-keeping. Adopting devices (ToolFS first, then `#agent`/`#cpu`/
//! AppResource per the ADR) own their job tables and lifecycles; this crate
//! pins the words they all agree on:
//!
//! - [`JobState`] — the lifecycle states and the pure legal-transition
//!   relation (`allocated → receiving → running → done | failed | aborted`).
//! - [`ErrorKind`] — the workspace-wide error taxonomy (exactly the nine ADR
//!   kinds) with each kind's default retryability.
//! - [`JobError`], [`JobStatus`], [`JobResult`] — the structured `status` and
//!   `result.json` report shapes (camelCase JSON, matching the
//!   `docs/toolfs.md` sketches).
//! - [`ResourceEnvelope`] — the shared `"wanix.resource": "v0"` spec envelope,
//!   designed to be `#[serde(flatten)]`ed into a device's `spec.json` shape.
//!
//! The grammar, the two-tier rule, and the idempotency rule live in
//! [ADR 0009]; the reference file layout lives in `docs/toolfs.md`.
//!
//! [ADR 0009]: ../../../docs/adrs/0009-job-protocol.md

/// Short crate purpose string for workspace smoke tests.
pub const CRATE_PURPOSE: &str = "job protocol vocabulary";

mod envelope;
mod report;
mod state;
mod taxonomy;

pub use envelope::{ResourceEnvelope, ResourceEnvelopeVersion, WANIX_RESOURCE_KEY};
pub use report::{JobError, JobResult, JobStatus};
pub use state::JobState;
pub use taxonomy::ErrorKind;
