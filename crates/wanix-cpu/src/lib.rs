//! Plan 9 cpu over the mesh: run a task next to the data, namespace from here.
//!
//! This crate is the synchronous, transport-agnostic core of the mesh's `#cpu`
//! capability — Plan 9's cpu(1) generalized to the open internet. A *caller*
//! dials a remote node, reverse-exports a **scoped, read-only-by-default**
//! sub-namespace, and the remote *acceptor* runs a task whose world *is* that
//! exported namespace. Compute travels to the data (or, equivalently, the data
//! is imported into the agent's world).
//!
//! # The shape of a job
//!
//! A job uses two role-sorted bidirectional streams:
//!
//! - **control** ([`CpuEvent`]): the acceptor delivers the job's stdout, stderr,
//!   and exit status over it after the task runs.
//! - **export** ([`Duplex`]): the caller runs its own [`wanix_9p::P9Server`] over
//!   a scoped [`ExportScope`]; the acceptor runs a [`wanix_9p_client::RemoteFs`]
//!   over the same stream and binds it as the task world.
//!
//! Over QUIC the two streams do not arrive in open order, so the caller writes a
//! 1-byte [`StreamRole`] discriminator first on each (control=0, export=1) and
//! the acceptor classifies each accepted stream by its first byte. Stream order
//! follows first-write, not open order; see [`role`].
//!
//! # The acceptor reuses the exact local launch
//!
//! [`run_job`] is `allocate_root` → `task.bind(world, ".", ".")` → configure →
//! `start`, byte-for-byte the local pattern, only with a remote world. The crate
//! never depends on a concrete task runtime (`qjs`, `wasm`): the caller of
//! [`run_job`] registers drivers on the [`wanix_task::TaskTable`] it passes in,
//! keeping the dependency direction honest.
//!
//! # Honest streaming and cancellation (v1)
//!
//! The current task model runs the guest to completion inside `start` and only
//! then has its buffered stdout, so v1 **delivers stdout/stderr/exit as a single
//! batch after `start` returns** rather than incrementally — stated plainly, not
//! faked. And because the task driver has no abort hook, a [`CpuEvent::Cancel`]
//! stops the caller *draining* the control stream; it does **not** stop the
//! remote computation. Incremental streaming and real remote cancellation are
//! named follow-ups, not implied capabilities.
//!
//! # Trust boundary
//!
//! The reverse export is a scoped sub-namespace ([`ExportScope`]): the job
//! subtree plus explicitly granted services, read-only by default — never the
//! caller's whole host root with client-controlled symlink following. Exporting
//! `#cpu`/`#task` for remote code execution stays local-trust / grant-allowlisted
//! until public auth lands; this crate provides the mechanism, not a public
//! policy.

mod acceptor;
mod caller;
mod error;
mod event;
mod role;
mod scope;
mod spec;
mod stdio;
mod transport;
mod wire;

pub use acceptor::run_job;
pub use caller::{CollectedOutput, JobOutput, drive_control, serve_export};
pub use error::{CpuError, CpuResult};
pub use event::{CpuEvent, MAX_EVENT_PAYLOAD};
pub use role::{ROLE_CONTROL, ROLE_EXPORT, StreamRole, read_role, write_role};
pub use scope::{ExportScope, GrantedService};
pub use spec::CpuJobSpec;
pub use transport::Duplex;
pub use wire::{MAX_FIELD_LEN, MAX_LIST_LEN, read_spec, write_spec};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix cpu: remote exec with namespace export";

#[cfg(test)]
mod tests {
    use super::CRATE_PURPOSE;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }
}
