//! Audited on-disk blob directory: the owner-private/atomic-write/fd-verified
//! boundary, reused verbatim for content-addressed storage.
//!
//! The Wasmtime module cache and the content-addressed store (`wanix-cas`'s
//! `LocalCasStore`) share one trust boundary: a local attacker must not be able
//! to pre-seed a hostile artifact that the victim later reads and trusts, and a
//! crashed or racing writer must never leave a torn artifact behind. That
//! boundary already exists, fd-verified, in [`crate::trust`]. Rather than
//! reimplement it (and risk diverging), [`AuditedBlobDir`] is a thin public
//! handle over the same [`crate::trust::read_trusted_artifact`] /
//! [`crate::trust::write_atomic`] functions the module cache uses, with the
//! artifact name supplied by the caller (a content hash) instead of a wasm
//! SHA-256.
//!
//! Reads through [`AuditedBlobDir`] are *not* `unsafe` to deserialize the way
//! the Wasmtime path is, because CAS content is plain bytes the caller
//! independently re-hashes; the trust boundary is still worth keeping so a
//! peer-user cannot inject bytes that masquerade as a locally-produced blob.

use std::path::{Path, PathBuf};

use crate::trust;

/// A handle to an owner-private directory used as a content-addressed blob
/// store, sharing the Wasmtime module cache's fd-verified read/write boundary.
///
/// All artifact names are caller-chosen (lowercase-hex content hashes); the
/// directory is created owner-only on first write and every read re-verifies
/// the leaf directory and artifact file through `O_NOFOLLOW` fds on Unix.
#[derive(Debug, Clone)]
pub struct AuditedBlobDir {
    dir: PathBuf,
}

impl AuditedBlobDir {
    /// Creates a handle for the owner-private blob directory at `dir`.
    ///
    /// The directory is not created until the first write; reads of a missing
    /// or untrusted directory return `None` (a miss), never an error.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Returns the on-disk directory backing this store.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Reads the artifact named `artifact`, returning its bytes only if the leaf
    /// directory and the file pass the fd-based owner-private checks.
    ///
    /// Returns `None` on any failed check, a missing file, or an I/O error, so a
    /// caller treats "not trusted" identically to "not present" and falls back
    /// to fetching the blob over the data plane.
    #[must_use]
    pub fn read(&self, artifact: &str) -> Option<Vec<u8>> {
        trust::read_trusted_artifact(&self.dir, artifact)
    }

    /// Returns whether `artifact` is present and passes the owner-private checks.
    #[must_use]
    pub fn has(&self, artifact: &str) -> bool {
        self.read(artifact).is_some()
    }

    /// Atomically writes `bytes` as the artifact named `artifact`.
    ///
    /// The directory is created owner-only if missing and verified through an
    /// `O_NOFOLLOW` fd before the write; the bytes are staged in a unique temp
    /// file and `rename`d into place, so a crash or concurrent writer never
    /// exposes a truncated artifact. `key_prefix` disambiguates concurrent
    /// writers' temp files.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the directory cannot be verified owner-private
    /// or the staged write/rename fails.
    pub fn write(&self, artifact: &str, bytes: &[u8], key_prefix: &str) -> std::io::Result<()> {
        trust::write_atomic(&self.dir, artifact, bytes, key_prefix)
    }
}
