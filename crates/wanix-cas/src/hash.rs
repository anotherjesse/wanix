//! BLAKE3 hashing of blob bytes into a [`ContentHash`].
//!
//! [`ContentHash`] (defined in `wanix-fs` to avoid a dependency cycle) only
//! carries 32 raw bytes; this module is where those bytes are actually
//! *computed*, over `blake3`. Keeping the one call site here means the whole
//! crate agrees on the digest algorithm, and the value lines up bit-for-bit
//! with the iroh-blobs `Hash` (also BLAKE3) the mesh data plane uses, so a
//! locally-ingested blob and a peer-fetched blob share one content address.

use wanix_fs::ContentHash;

/// Computes the BLAKE3 [`ContentHash`] of `bytes`.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> ContentHash {
    let digest = blake3::hash(bytes);
    ContentHash::from_bytes(*digest.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_bytes_hash_equal() {
        assert_eq!(hash_bytes(b"abc"), hash_bytes(b"abc"));
    }

    #[test]
    fn distinct_bytes_hash_distinct() {
        assert_ne!(hash_bytes(b"abc"), hash_bytes(b"abd"));
    }

    #[test]
    fn matches_known_blake3_vector() {
        // BLAKE3 of the empty input is a fixed, well-known digest; this anchors
        // the wrapper to the real algorithm rather than an accidental stand-in.
        let empty = hash_bytes(b"");
        assert_eq!(
            empty.to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }
}
