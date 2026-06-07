//! The [`ContentStore`] trait: the synchronous, transport-free contract for a
//! content-addressed blob store (venti's archival store, Wanix-shaped).
//!
//! A store maps a [`ContentHash`] to immutable bytes. Putting bytes returns
//! their hash (so dedup is automatic: equal bytes yield one entry); getting by
//! hash returns the bytes or [`CasError::NotFound`]. The trait is deliberately
//! `Send + Sync` and synchronous so the sync 9P core and the `#cas` device can
//! depend on it without any async leakage; the async, network-fetching
//! implementation (`IrohCasStore`) lives in `wanix-mesh` and is reached through
//! a blocking bridge, exactly like the 9P client.

use wanix_fs::ContentHash;

use crate::hash::hash_bytes;

/// Maximum blob size a store will accept or return by default, in bytes.
///
/// `get_bytes`-style whole-blob reads load the entire blob into memory, so an
/// unbounded blob (especially one named by a *hostile* ticket) is an
/// out-of-memory / disk-fill vector. Stores and capsule materialization clamp
/// to this ceiling; callers that genuinely need larger blobs opt in explicitly.
/// 256 MiB comfortably covers a rootfs image while bounding a single hostile
/// allocation.
pub const MAX_BLOB_SIZE: usize = 256 * 1024 * 1024;

/// The error surface of a [`ContentStore`].
#[derive(Debug)]
pub enum CasError {
    /// No blob with the requested [`ContentHash`] is available.
    NotFound,
    /// A blob exceeded [`MAX_BLOB_SIZE`] (the put/get refused to load it).
    TooLarge {
        /// The offending blob's length in bytes.
        len: usize,
    },
    /// The bytes fetched for a hash did not re-hash to that hash.
    ///
    /// This is the end-to-end verification failure: a store (or a peer) returned
    /// bytes that do not match the content address, so they are rejected rather
    /// than trusted.
    HashMismatch {
        /// The hash that was requested.
        requested: ContentHash,
        /// The hash the returned bytes actually produced.
        actual: ContentHash,
    },
    /// An underlying I/O or backend failure, described for diagnostics.
    Backend(String),
}

impl std::fmt::Display for CasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "content not found"),
            Self::TooLarge { len } => {
                write!(
                    f,
                    "blob of {len} bytes exceeds the {MAX_BLOB_SIZE}-byte cap"
                )
            }
            Self::HashMismatch { requested, actual } => write!(
                f,
                "hash mismatch: requested {}, got {}",
                requested.to_hex(),
                actual.to_hex()
            ),
            Self::Backend(message) => write!(f, "cas backend error: {message}"),
        }
    }
}

impl std::error::Error for CasError {}

/// Result type for [`ContentStore`] operations.
pub type CasResult<T> = Result<T, CasError>;

/// A synchronous content-addressed blob store.
///
/// Implementations must be `Send + Sync` so a store can be shared behind an
/// `Arc` across the 9P server's per-connection threads and the `#cas` device.
pub trait ContentStore: Send + Sync {
    /// Stores `bytes` and returns their [`ContentHash`].
    ///
    /// Equal byte sequences deduplicate to one entry. Implementations must
    /// refuse blobs larger than [`MAX_BLOB_SIZE`] with [`CasError::TooLarge`].
    ///
    /// # Errors
    ///
    /// Returns [`CasError::TooLarge`] when `bytes` exceeds the cap and
    /// [`CasError::Backend`] on a storage failure.
    fn put(&self, bytes: &[u8]) -> CasResult<ContentHash>;

    /// Returns the bytes stored under `hash`, verifying they re-hash to it.
    ///
    /// # Errors
    ///
    /// Returns [`CasError::NotFound`] when the hash is absent,
    /// [`CasError::TooLarge`] when the stored blob exceeds the cap, and
    /// [`CasError::HashMismatch`] when the returned bytes do not match `hash`.
    fn get(&self, hash: &ContentHash) -> CasResult<Vec<u8>>;

    /// Returns whether a blob for `hash` is present locally.
    ///
    /// This is a cheap existence probe (the `#cas/have/<hash>` query); it does
    /// not load or verify the bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CasError::Backend`] when presence cannot be determined.
    fn has(&self, hash: &ContentHash) -> CasResult<bool>;
}

/// Verifies that `bytes` hash to `expected`, returning [`CasError::HashMismatch`]
/// otherwise.
///
/// Every store read path runs this so that no implementation — local disk or a
/// hostile remote peer — can return bytes that do not match the content address.
///
/// # Errors
///
/// Returns [`CasError::HashMismatch`] when the recomputed hash differs.
pub fn verify_hash(bytes: &[u8], expected: &ContentHash) -> CasResult<()> {
    let actual = hash_bytes(bytes);
    if &actual == expected {
        Ok(())
    } else {
        Err(CasError::HashMismatch {
            requested: *expected,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_accepts_matching_bytes() {
        let bytes = b"venti score";
        let hash = hash_bytes(bytes);
        assert!(verify_hash(bytes, &hash).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_bytes() {
        let hash = hash_bytes(b"original");
        let err = verify_hash(b"tampered", &hash).unwrap_err();
        assert!(matches!(err, CasError::HashMismatch { .. }));
    }

    #[test]
    fn error_messages_are_human_readable() {
        assert!(CasError::NotFound.to_string().contains("not found"));
        assert!(CasError::TooLarge { len: 999 }.to_string().contains("999"));
    }
}
