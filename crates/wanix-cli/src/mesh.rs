//! `wanix-rust mesh-serve` and the `iroh://` mount scheme.
//!
//! These wire the `wanix-mesh` transport into the CLI: `mesh-serve` binds an iroh
//! endpoint from the persisted node identity and exports a host directory (with
//! optional default-deny grants) as 9P over QUIC, printing the node's id and
//! dialable ticket. The `iroh://` mount scheme teaches the existing `mount-*`
//! verbs to dial a peer over QUIC instead of TCP, so a remote namespace imports
//! over the open internet exactly as it does over a loopback socket.
//!
//! The peer identity is always the cryptographically verified key from the QUIC
//! connection, never a client-claimed `uname`.

mod grant;
pub(crate) mod mounts;
pub(crate) mod resource;
mod serve;
mod ticket;

pub(crate) use mounts::{bind_mesh_mounts, bind_mesh_mounts_into};
pub(crate) use serve::{parse_mesh_serve_command, run_mesh_serve_streaming};
#[cfg(test)]
pub(crate) use ticket::dial_iroh_remote_as;
pub(crate) use ticket::{
    CLI_MESH_MOUNT_DEADLINE, IROH_SCHEME, IrohMount, MeshTicket, ProbeOutcome, dial_iroh_remote,
    dialer_identity, probe_iroh,
};
