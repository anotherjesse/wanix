//! Content-addressing primitive shared by the filesystem and data-plane crates.
//!
//! [`ContentHash`] is a BLAKE3 digest newtype. It lives in `wanix-fs` — the
//! crate at the bottom of the dependency graph — so that the [`FileSystem`]
//! offload hook ([`FileSystem::content_hash`](crate::FileSystem::content_hash)),
//! the content-addressed store crate (`wanix-cas`), and the mesh data plane
//! (`wanix-mesh`) can all name the same type without any of them depending on
//! each other or on a hashing crate. The hash *value* is computed by whoever
//! holds the bytes (`wanix-cas` over `blake3`, or `wanix-mesh` over the
//! iroh-blobs `Hash`, which is itself BLAKE3); this type only carries and
//! formats the 32 raw bytes.

use crate::{FsError, FsResult};

/// Number of raw bytes in a BLAKE3 [`ContentHash`].
pub const CONTENT_HASH_LEN: usize = 32;

/// Number of lowercase-hex characters in a [`ContentHash`] string form.
pub const CONTENT_HASH_HEX_LEN: usize = CONTENT_HASH_LEN * 2;

/// A BLAKE3 content hash: the stable, verifiable name of a blob of bytes.
///
/// Two byte sequences share a [`ContentHash`] if and only if they are equal
/// (modulo BLAKE3's cryptographic collision resistance), which is exactly the
/// dedup and end-to-end-verification property the data plane relies on. The
/// inner 32 bytes are the raw digest, *not* hex; use [`ContentHash::to_hex`] for
/// the path/string form and [`ContentHash::from_hex`] to parse it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash([u8; CONTENT_HASH_LEN]);

impl ContentHash {
    /// Wraps 32 raw digest bytes as a [`ContentHash`].
    #[must_use]
    pub const fn from_bytes(bytes: [u8; CONTENT_HASH_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the 32 raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; CONTENT_HASH_LEN] {
        &self.0
    }

    /// Returns the digest as a 64-character lowercase-hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(CONTENT_HASH_HEX_LEN);
        for byte in &self.0 {
            // Two lowercase hex nibbles per byte, no allocation per byte.
            out.push(nibble_to_hex(byte >> 4));
            out.push(nibble_to_hex(byte & 0x0f));
        }
        out
    }

    /// Parses a 64-character lowercase-hex string into a [`ContentHash`].
    ///
    /// # Errors
    ///
    /// Returns [`FsError::InvalidPath`] when `hex` is not exactly
    /// [`CONTENT_HASH_HEX_LEN`] lowercase-hex characters, so a hostile or
    /// malformed `#cas/<hash>` path component is rejected before any lookup.
    pub fn from_hex(hex: &str) -> FsResult<Self> {
        if hex.len() != CONTENT_HASH_HEX_LEN {
            return Err(FsError::InvalidPath(hex.to_owned()));
        }
        let bytes = hex.as_bytes();
        let mut out = [0u8; CONTENT_HASH_LEN];
        for (index, slot) in out.iter_mut().enumerate() {
            let high = hex_to_nibble(bytes[index * 2])
                .ok_or_else(|| FsError::InvalidPath(hex.to_owned()))?;
            let low = hex_to_nibble(bytes[index * 2 + 1])
                .ok_or_else(|| FsError::InvalidPath(hex.to_owned()))?;
            *slot = (high << 4) | low;
        }
        Ok(Self(out))
    }
}

/// Maps a 4-bit value (0..=15) to its lowercase-hex character.
const fn nibble_to_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'a' + (nibble - 10)) as char,
    }
}

/// Maps a lowercase-hex ASCII byte to its 4-bit value, rejecting anything else.
///
/// Uppercase hex is intentionally rejected so a hash has exactly one canonical
/// string form (`to_hex` only ever emits lowercase), which keeps `#cas/<hash>`
/// paths and blob tickets one-to-one with the underlying bytes.
const fn hex_to_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = [0xab; CONTENT_HASH_LEN];
        let hash = ContentHash::from_bytes(bytes);
        let hex = hash.to_hex();
        assert_eq!(hex.len(), CONTENT_HASH_HEX_LEN);
        assert_eq!(hex, "ab".repeat(CONTENT_HASH_LEN));
        assert_eq!(ContentHash::from_hex(&hex).unwrap(), hash);
    }

    #[test]
    fn distinct_nibbles_round_trip() {
        let mut bytes = [0u8; CONTENT_HASH_LEN];
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = index as u8;
        }
        let hash = ContentHash::from_bytes(bytes);
        assert_eq!(ContentHash::from_hex(&hash.to_hex()).unwrap(), hash);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(ContentHash::from_hex("ab").is_err());
        assert!(ContentHash::from_hex(&"a".repeat(CONTENT_HASH_HEX_LEN + 1)).is_err());
    }

    #[test]
    fn rejects_non_hex_and_uppercase() {
        assert!(ContentHash::from_hex(&"g".repeat(CONTENT_HASH_HEX_LEN)).is_err());
        assert!(ContentHash::from_hex(&"A".repeat(CONTENT_HASH_HEX_LEN)).is_err());
    }
}
