//! Mesh transport for Wanix: FileSystem-over-iroh QUIC, the only async crate.
//!
//! This crate is the network edge of the Wanix mesh. It binds one
//! [`iroh::Endpoint`] per node from the persisted [`wanix_id::NodeIdentity`]
//! secret key and exports a Wanix namespace over QUIC on two control-plane
//! ALPNs:
//!
//! - the **native** `wanix-mesh-wire` plane ([`WANIX_FS_ALPN`]) — the
//!   Wanix↔Wanix mesh path, a hand-rolled `postcard` frame carrying the full
//!   [`wanix_fs::FileSystem`] trait with typed errors and one bidi stream per op
//!   / per open file. [`MeshDialer::dial_native`] imports a peer as a
//!   [`NativeFs`]; this is the default mesh import wire.
//! - the **9P** plane ([`WANIX_9P_ALPN`]) — the foreign edge (Linux/v86/QEMU,
//!   external 9P tools, the cockpit). [`MeshDialer::dial`] imports a peer as a
//!   [`wanix_9p_client::RemoteFs`]; the synchronous 9P core
//!   ([`wanix_9p::P9Server`] and [`wanix_9p_client::RemoteFs`]) is reused
//!   **unchanged**.
//!
//! Both planes ride the one identity-bound endpoint; the async/sync boundary is
//! bridged here and nowhere else.
//!
//! # Layering
//!
//! Per the dependency-direction rule, iroh and tokio live only in this crate.
//! Core `wanix-fs`/`wanix-vfs`/`wanix-protocol`/`wanix-9p`/`wanix-task` stay
//! free of any async runtime. The bridge has two directions:
//!
//! - **Inbound** ([`handler`]): the [`P9ProtocolHandler`] reads the verified peer
//!   id from the QUIC handshake, checks the grant policy, and for each accepted
//!   bidi stream runs [`wanix_9p::P9Server::serve_stream`] inside
//!   `tokio::task::spawn_blocking`, unchanged.
//! - **Outbound** ([`dialer`]): [`MeshDialer::dial`] connects, opens a bidi
//!   stream, wraps it in a [`BlockingDuplex`] that drives the async halves on a
//!   **held** runtime [`tokio::runtime::Handle`] (never `block_on` on a worker),
//!   and hands it to [`wanix_9p_client::RemoteFs`].
//!
//! # Identity is the transport peer
//!
//! Authorization is keyed by the cryptographic [`wanix_id::PeerId`] read from the
//! connection, never a client-claimed `uname`. A grant is a bind: the served root
//! a peer attaches is a [`wanix_vfs::SubtreeFs`] chosen by the
//! [`wanix_id::AttachPolicy`].

mod cas;
mod cpu;
mod dialer;
mod duplex;
mod error;
mod handler;
mod identity;
mod node;
mod plumb;
mod wire_handler;

pub use cas::{IrohCasStore, blob_hash, blobs_protocol, content_hash};
pub use cpu::{CpuAcceptor, CpuDialer, CpuJobReport, TaskTableFactory, WANIX_CPU_ALPN};
pub use dialer::{IrohStreamFactory, MeshDialer};
pub use duplex::{BlockingDuplex, BlockingReader, BlockingWriter};
pub use error::{MeshError, MeshResult};
pub use handler::{P9ProtocolHandler, ServeConfig};
pub use identity::{endpoint_id_for, peer_id_for, secret_key_for};
pub use node::{DEFAULT_OP_DEADLINE, MeshNode};
pub use plumb::{GOSSIP_ALPN, GossipPlumbPort};
pub use wire_handler::{NativeFsHandler, NativeServeConfig};

/// Re-export of the native-wire client `FileSystem`, the import half of the
/// native mesh wire (the analog of [`wanix_9p_client::RemoteFs`] on the 9P
/// plane). [`MeshDialer::dial_native`] returns one of these bound over an iroh
/// QUIC connection.
pub use wanix_mesh_wire::NativeFs;

/// Re-export of iroh's dialable peer address, the mesh "ticket" form.
pub use iroh::EndpointAddr;
/// Re-export of iroh's verified endpoint identity (an ed25519 public key).
pub use iroh::EndpointId;
/// Re-export of the iroh-blobs ALPN, the data-plane wire contract registered on
/// the shared [`MeshNode`] [`Router`](iroh::protocol::Router) endpoint.
pub use iroh_blobs::ALPN as BLOBS_ALPN;

/// The ALPN protocol identifier for the Wanix 9P control plane over QUIC.
///
/// Both the inbound [`P9ProtocolHandler`] and the outbound [`MeshDialer`] speak
/// exactly this ALPN; it is the wire contract that selects the 9P plane on a
/// shared endpoint.
pub const WANIX_9P_ALPN: &[u8] = b"wanix/9p/1";

/// The ALPN protocol identifier for the Wanix *native* FileSystem control plane
/// over QUIC.
///
/// This is the Wanix↔Wanix mesh wire (`wanix-mesh-wire`): a hand-rolled,
/// length-prefixed `postcard` frame, one bidi stream per op / per open file,
/// carrying the full [`wanix_fs::FileSystem`] trait with typed errors and a
/// per-connection principal bound from `remote_id()`. It is a distinct plane
/// from [`WANIX_9P_ALPN`]; 9P stays as the foreign edge (Linux/v86/QEMU,
/// external 9P tools, the cockpit), the native wire is the mesh path only. A
/// node can advertise both ALPNs during the transition, so it speaks both wires.
pub const WANIX_FS_ALPN: &[u8] = b"wanix/fs/1";

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix mesh transport over iroh QUIC";

#[cfg(test)]
mod tests {
    use super::{CRATE_PURPOSE, WANIX_9P_ALPN, WANIX_FS_ALPN};

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn alpn_is_the_wanix_9p_contract() {
        assert_eq!(WANIX_9P_ALPN, b"wanix/9p/1");
    }

    #[test]
    fn native_alpn_is_the_wanix_fs_contract() {
        assert_eq!(WANIX_FS_ALPN, b"wanix/fs/1");
        // The native plane is a distinct ALPN from 9P; a node advertising both
        // must not collide them.
        assert_ne!(WANIX_FS_ALPN, WANIX_9P_ALPN);
    }
}
