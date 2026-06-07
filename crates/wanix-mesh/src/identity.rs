//! The bridge between Wanix identity and iroh identity.
//!
//! Wanix names a node by an ed25519 keypair ([`wanix_id::NodeIdentity`]) and a
//! verified peer by its raw public key ([`wanix_id::PeerId`]). iroh names the
//! same things with [`iroh::SecretKey`] and [`iroh::EndpointId`] over the same
//! ed25519 primitive. This module converts between the two as raw 32-byte keys,
//! exactly as the blueprint requires (no reliance on iroh re-exporting
//! `ed25519-dalek`, which is a private pre-release).

use iroh::{EndpointId, SecretKey};
use wanix_id::{NodeIdentity, PeerId};

/// Returns the iroh [`SecretKey`] for a Wanix [`NodeIdentity`].
///
/// The 32-byte ed25519 seed is shared verbatim, so the iroh endpoint's identity
/// is byte-for-byte the persisted Wanix node identity and its public key is the
/// node's [`PeerId`].
#[must_use]
pub fn secret_key_for(identity: &NodeIdentity) -> SecretKey {
    SecretKey::from_bytes(&identity.to_secret_bytes())
}

/// Returns the Wanix [`PeerId`] for an iroh-verified [`EndpointId`].
///
/// The endpoint id is the peer's ed25519 public key, read from the QUIC
/// handshake's TLS certificate; this is the cryptographic identity the
/// [`wanix_id::GrantTable`] is keyed by — never a client-claimed `uname`.
#[must_use]
pub fn peer_id_for(endpoint_id: EndpointId) -> PeerId {
    PeerId::from_bytes(*endpoint_id.as_bytes())
}

/// Returns the iroh [`EndpointId`] (public key) of a Wanix [`PeerId`].
///
/// # Errors
///
/// Returns an error string when the 32 bytes are not a valid ed25519 point.
pub fn endpoint_id_for(peer: PeerId) -> Result<EndpointId, String> {
    EndpointId::from_bytes(peer.as_bytes()).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_identity_round_trips_through_iroh_secret_key() {
        let identity = NodeIdentity::from_secret_bytes([7u8; 32]);
        let secret = secret_key_for(&identity);
        assert_eq!(secret.to_bytes(), identity.to_secret_bytes());
    }

    #[test]
    fn iroh_public_key_matches_the_wanix_peer_id() {
        let identity = NodeIdentity::from_secret_bytes([42u8; 32]);
        let secret = secret_key_for(&identity);
        let derived = peer_id_for(secret.public());
        assert_eq!(derived, identity.peer_id());
    }

    #[test]
    fn peer_id_round_trips_through_endpoint_id() {
        let identity = NodeIdentity::from_secret_bytes([9u8; 32]);
        let peer = identity.peer_id();
        let endpoint_id = endpoint_id_for(peer).unwrap();
        assert_eq!(peer_id_for(endpoint_id), peer);
    }
}
