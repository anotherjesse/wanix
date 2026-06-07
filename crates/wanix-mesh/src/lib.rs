//! Mesh transport for Wanix: 9P over iroh QUIC, the only async crate.
//!
//! This crate is the network edge of the Wanix mesh. It binds one
//! [`iroh::Endpoint`] per node from the persisted [`wanix_id::NodeIdentity`]
//! secret key, exports a Wanix namespace as 9P over QUIC under ALPN
//! [`WANIX_9P_ALPN`], and dials peers to import their namespaces as a
//! [`wanix_9p_client::RemoteFs`]. The synchronous 9P core
//! ([`wanix_9p::P9Server`] and [`wanix_9p_client::RemoteFs`]) is reused
//! **unchanged**; the async/sync boundary is bridged here and nowhere else.
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

mod dialer;
mod duplex;
mod error;
mod handler;
mod identity;
mod node;

pub use dialer::MeshDialer;
pub use duplex::{BlockingDuplex, BlockingReader, BlockingWriter};
pub use error::{MeshError, MeshResult};
pub use handler::{P9ProtocolHandler, ServeConfig};
pub use identity::{endpoint_id_for, peer_id_for, secret_key_for};
pub use node::{DEFAULT_OP_DEADLINE, MeshNode};

/// Re-export of iroh's dialable peer address, the mesh "ticket" form.
pub use iroh::EndpointAddr;
/// Re-export of iroh's verified endpoint identity (an ed25519 public key).
pub use iroh::EndpointId;

/// The ALPN protocol identifier for the Wanix 9P control plane over QUIC.
///
/// Both the inbound [`P9ProtocolHandler`] and the outbound [`MeshDialer`] speak
/// exactly this ALPN; it is the wire contract that selects the 9P plane on a
/// shared endpoint.
pub const WANIX_9P_ALPN: &[u8] = b"wanix/9p/1";

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix mesh transport over iroh QUIC";

#[cfg(test)]
mod tests {
    use super::{CRATE_PURPOSE, WANIX_9P_ALPN};

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn alpn_is_the_wanix_9p_contract() {
        assert_eq!(WANIX_9P_ALPN, b"wanix/9p/1");
    }
}
