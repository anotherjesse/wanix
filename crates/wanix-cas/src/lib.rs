//! Content-addressed data plane for Wanix — venti, made Wanix-shaped.
//!
//! This crate is the synchronous, iroh-free core of the mesh's bulk data plane.
//! It owns:
//!
//! - [`ContentStore`]: the `Send + Sync` trait for a blob store keyed by
//!   [`wanix_fs::ContentHash`] (BLAKE3). Equal bytes deduplicate to one entry,
//!   reads are end-to-end verified, and blob size is capped against hostile
//!   tickets.
//! - [`LocalCasStore`]: an on-disk store reusing `wanix-module-cache`'s audited
//!   owner-private/atomic-write/fd-verified boundary verbatim.
//! - [`CasDevice`]: the `#cas` service filesystem — `#cas/<hash>` reads a blob,
//!   `#cas/ingest` is write-then-read-hash, and `#cas/have/<hash>` probes
//!   presence — so blobs are operable as ordinary Wanix files.
//! - [`WorldManifest`] / [`Capsule`]: a deterministic, content-addressed view of
//!   a directory tree (a "world"), where each file is a blob, the sorted
//!   manifest is itself a blob whose hash *is* the capsule id, and
//!   [`WorldManifest::materialize`] re-applies the path-safety guard and caps
//!   blob size + manifest fan-out before writing anything to disk.
//!
//! The async, network-fetching store (`IrohCasStore`, over iroh-blobs) lives in
//! `wanix-mesh`; it implements the same [`ContentStore`] trait through a
//! blocking bridge, so nothing here depends on iroh or tokio.

mod capsule;
mod casfs;
mod device;
mod hash;
mod local;
mod store;

pub use capsule::{
    CAPSULE_MANIFEST_MAX_BYTES, Capsule, MAX_MANIFEST_ENTRIES, ManifestEntry, MaterializeError,
    MaterializeResult, MaterializeStats, WorldManifest,
};
pub use casfs::CasFs;
pub use device::CasDevice;
pub use hash::hash_bytes;
pub use local::{CAS_DIR_ENV, LocalCasStore};
pub use store::{CasError, CasResult, ContentStore, MAX_BLOB_SIZE, verify_hash};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix content-addressed data plane (venti)";

#[cfg(test)]
mod tests {
    use super::CRATE_PURPOSE;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }
}
