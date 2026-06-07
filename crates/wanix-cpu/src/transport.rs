//! The transport contract the CPU acceptor and caller speak over.
//!
//! Like the 9P client, `wanix-cpu` is transport-agnostic: a job's control and
//! export streams are just blocking byte streams. The acceptor's export world is
//! a [`wanix_9p_client::RemoteFs`], so the export stream must satisfy that
//! crate's [`Duplex`] contract — which this module re-exports so callers need a
//! single import. The QUIC role discrimination and stream pairing live in the
//! transport layer (`wanix-mesh`); this crate sees already-sorted streams.

/// Re-export of the 9P client's blocking byte-stream contract.
///
/// Any `Read + Write + Send` type is a `Duplex`. The export stream handed to
/// [`crate::run_job`] must be one, because the acceptor runs a
/// [`wanix_9p_client::RemoteFs`] over it.
pub use wanix_9p_client::Duplex;
