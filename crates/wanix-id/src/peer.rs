use std::fmt;

/// A verified peer identity: the raw 32-byte ed25519 public key of the node on
/// the other end of a connection.
///
/// In the mesh this is read from an authenticated QUIC connection; in tests and
/// over plain TCP it is supplied explicitly. Either way it is the
/// cryptographic identity the [`crate::GrantTable`] is keyed by — never a
/// client-claimed `uname`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId([u8; 32]);

impl PeerId {
    /// Creates a peer identity from a raw 32-byte ed25519 public key.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 32-byte ed25519 public key.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the lowercase hex encoding of the public key.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(hex_nibble(byte >> 4));
            out.push(hex_nibble(byte & 0x0f));
        }
        out
    }
}

fn hex_nibble(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + (value - 10)) as char,
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PeerId").field(&self.to_hex()).finish()
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::PeerId;

    #[test]
    fn hex_round_trips_through_bytes() {
        let bytes = [
            0x00, 0x01, 0x02, 0x0f, 0x10, 0xab, 0xcd, 0xef, 0xff, 0x42, 0x00, 0x01, 0x02, 0x03,
            0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11,
            0x12, 0x13, 0x14, 0x15,
        ];
        let peer = PeerId::from_bytes(bytes);
        assert_eq!(peer.as_bytes(), &bytes);
        assert_eq!(&peer.to_hex()[..8], "0001020f");
        assert_eq!(peer.to_hex().len(), 64);
        assert_eq!(format!("{peer}"), peer.to_hex());
    }

    #[test]
    fn equality_is_by_key() {
        let a = PeerId::from_bytes([1u8; 32]);
        let b = PeerId::from_bytes([1u8; 32]);
        let c = PeerId::from_bytes([2u8; 32]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
