//! [`LocalCasStore`]: an on-disk [`ContentStore`] over the module cache's
//! audited owner-private/atomic-write/fd-verified boundary.
//!
//! Each blob is stored as one file named by its lowercase-hex [`ContentHash`] in
//! an owner-private directory. Writes are atomic (temp-file + rename) and reads
//! are fd-verified through `wanix-module-cache`'s [`AuditedBlobDir`], so a
//! peer-user cannot pre-seed a blob the victim later trusts. Every read also
//! re-hashes the bytes before returning them, so even a same-user-corrupted
//! file is rejected rather than served under the wrong content address.

use wanix_fs::ContentHash;
use wanix_module_cache::{AuditedBlobDir, owner_private_cache_dir};

use crate::hash::hash_bytes;
use crate::store::{CasError, CasResult, ContentStore, MAX_BLOB_SIZE, verify_hash};

/// Environment variable that overrides the default `LocalCasStore` directory.
pub const CAS_DIR_ENV: &str = "WANIX_CAS_DIR";

/// Per-user cache subdirectory under which blobs live by default.
const CAS_SUBDIR: &str = "cas-store";

/// An on-disk, owner-private content-addressed store.
#[derive(Debug, Clone)]
pub struct LocalCasStore {
    dir: AuditedBlobDir,
}

impl LocalCasStore {
    /// Opens a store at the per-user owner-private default location.
    ///
    /// The location is `$WANIX_CAS_DIR` when set, else a `cas-store`
    /// subdirectory of the platform per-user cache root. The directory is
    /// created owner-only on the first write.
    #[must_use]
    pub fn open_default() -> Self {
        Self::open(owner_private_cache_dir(CAS_DIR_ENV, CAS_SUBDIR))
    }

    /// Opens a store rooted at the explicit directory `dir`.
    ///
    /// The caller asserts `dir`'s ancestors are owner-private; the leaf
    /// directory and every artifact are still fd-verified on each read.
    #[must_use]
    pub fn open(dir: std::path::PathBuf) -> Self {
        Self {
            dir: AuditedBlobDir::new(dir),
        }
    }

    /// Returns the on-disk directory backing this store.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        self.dir.path()
    }
}

impl ContentStore for LocalCasStore {
    fn put(&self, bytes: &[u8]) -> CasResult<ContentHash> {
        if bytes.len() > MAX_BLOB_SIZE {
            return Err(CasError::TooLarge { len: bytes.len() });
        }
        let hash = hash_bytes(bytes);
        let name = hash.to_hex();
        // A content-addressed blob is immutable: if a trusted copy already
        // exists, the write is a no-op and we avoid rewriting identical bytes.
        if self.dir.has(&name) {
            return Ok(hash);
        }
        // The first two hex chars of the hash disambiguate concurrent writers'
        // temp files, mirroring the module cache's key-prefix scheme.
        let key_prefix = &name[..2];
        self.dir
            .write(&name, bytes, key_prefix)
            .map_err(|err| CasError::Backend(err.to_string()))?;
        Ok(hash)
    }

    fn get(&self, hash: &ContentHash) -> CasResult<Vec<u8>> {
        let name = hash.to_hex();
        let bytes = self.dir.read(&name).ok_or(CasError::NotFound)?;
        if bytes.len() > MAX_BLOB_SIZE {
            return Err(CasError::TooLarge { len: bytes.len() });
        }
        // Re-hash on the way out: a same-user-corrupted file must not be served
        // under a content address it no longer matches.
        verify_hash(&bytes, hash)?;
        Ok(bytes)
    }

    fn has(&self, hash: &ContentHash) -> CasResult<bool> {
        Ok(self.dir.has(&hash.to_hex()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(label: &str) -> LocalCasStore {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("wanix-cas-{label}-{}-{nonce}", std::process::id()));
        LocalCasStore::open(dir)
    }

    #[test]
    fn put_then_get_round_trips() {
        let store = temp_store("round-trip");
        let hash = store.put(b"hello venti").unwrap();
        assert_eq!(store.get(&hash).unwrap(), b"hello venti");
        std::fs::remove_dir_all(store.path()).ok();
    }

    #[test]
    fn put_is_idempotent_and_dedups() {
        let store = temp_store("dedup");
        let a = store.put(b"same bytes").unwrap();
        let b = store.put(b"same bytes").unwrap();
        assert_eq!(a, b);
        std::fs::remove_dir_all(store.path()).ok();
    }

    #[test]
    fn get_missing_is_not_found() {
        let store = temp_store("missing");
        let absent = hash_bytes(b"never stored");
        assert!(matches!(store.get(&absent), Err(CasError::NotFound)));
        assert!(!store.has(&absent).unwrap());
        std::fs::remove_dir_all(store.path()).ok();
    }

    #[test]
    fn has_reports_presence() {
        let store = temp_store("has");
        let hash = store.put(b"present").unwrap();
        assert!(store.has(&hash).unwrap());
        std::fs::remove_dir_all(store.path()).ok();
    }

    #[test]
    fn corrupted_file_is_rejected_on_get() {
        let store = temp_store("corrupt");
        let hash = store.put(b"original bytes").unwrap();
        // Overwrite the on-disk blob with different bytes under the same name.
        let path = store.path().join(hash.to_hex());
        std::fs::write(&path, b"tampered bytes").unwrap();
        assert!(matches!(
            store.get(&hash),
            Err(CasError::HashMismatch { .. })
        ));
        std::fs::remove_dir_all(store.path()).ok();
    }
}
