//! Node identity and default-deny capability grants for the Rust-native Wanix
//! mesh.
//!
//! This crate is deliberately transport-free and iroh-free: identity is an
//! ed25519 keypair ([`NodeIdentity`]) and a verified peer is a raw 32-byte
//! public key ([`PeerId`]). That keeps the trust boundary testable over a local
//! pipe or a TCP loopback, where the peer identity is supplied explicitly,
//! before any QUIC transport exists to verify it cryptographically.
//!
//! Authorization is a pure function from a verified peer plus an attach name to
//! an [`Authorization`] (a scoped root [`wanix_vfs::SubtreeFs`] plus its
//! [`wanix_vfs::Rights`]). The [`GrantTable`] is **default-deny**: a peer with
//! no matching [`Grant`] gets nothing. A grant is, concretely, "this peer may
//! attach this subtree with these rights" — a capability that becomes a bind.

mod grant;
mod identity;
mod peer;
mod policy;

pub use grant::{Authorization, Grant, GrantTable};
pub use identity::{NodeIdentity, NodeIdentityError};
pub use peer::PeerId;
pub use policy::{AttachPolicy, GrantTablePolicy};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix node identity and capability grants";

#[cfg(test)]
mod tests {
    use super::CRATE_PURPOSE;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }
}
