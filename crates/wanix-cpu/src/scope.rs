//! Building the caller's scoped, read-only-by-default export namespace.
//!
//! The export stream carries the caller's *own* `P9Server`, but it must **not**
//! serve the caller's whole host root. The cpu correction is explicit: the
//! reverse export is a scoped sub-namespace — the job's working subtree plus
//! explicitly granted services — and read-only by default, never
//! `services_namespace_for_root`'s entire host root with client-controlled
//! symlink following.
//!
//! [`ExportScope`] is the declarative description of that sub-namespace, and
//! [`ExportScope::into_root`] materializes it as a single
//! [`wanix_fs::FileSystem`] suitable to hand to a [`wanix_9p::P9Server`]. The job
//! subtree is re-rooted with [`wanix_vfs::SubtreeFs`]; each granted service is
//! re-rooted and rights-gated the same way and bound under its `#`-device name.

use std::sync::Arc;

use wanix_fs::{FileSystem, FsResult};
use wanix_vfs::{BindOptions, Namespace, Rights, SubtreeFs};

/// One service the caller explicitly grants into the export scope.
///
/// A granted service is a backing [`FileSystem`] re-rooted at `prefix` with
/// `rights` and bound at `mount` (e.g. `#kv`) in the exported namespace. Grants
/// are opt-in: nothing the caller does not name is reachable from node Y.
#[derive(Clone)]
pub struct GrantedService {
    /// Namespace path the service is bound at in the export scope (e.g. `#kv`).
    pub mount: String,
    /// The backing filesystem the service re-roots.
    pub backing: Arc<dyn FileSystem>,
    /// The backing-relative prefix the service is scoped to (`.` for its root).
    pub prefix: String,
    /// The rights gate applied to the service (read-only unless write is granted).
    pub rights: Rights,
}

impl GrantedService {
    /// Grants `backing` at `mount`, re-rooted at `prefix` with `rights`.
    #[must_use]
    pub fn new(
        mount: impl Into<String>,
        backing: Arc<dyn FileSystem>,
        prefix: impl Into<String>,
        rights: Rights,
    ) -> Self {
        Self {
            mount: mount.into(),
            backing,
            prefix: prefix.into(),
            rights,
        }
    }
}

/// The declarative description of a caller's export sub-namespace.
///
/// Build it with [`Self::new`] (the job subtree, read-only by default), opt into
/// write with [`Self::writable`], and add explicitly granted services with
/// [`Self::grant`]. [`Self::into_root`] turns it into the single root the export
/// `P9Server` serves.
pub struct ExportScope {
    backing: Arc<dyn FileSystem>,
    prefix: String,
    rights: Rights,
    services: Vec<GrantedService>,
}

impl ExportScope {
    /// Scopes the export to `backing` re-rooted at `prefix`, **read-only**.
    ///
    /// Read-only is the default per the cpu correction: a remote job gets read
    /// access to the job subtree unless the caller deliberately opts into write.
    #[must_use]
    pub fn new(backing: Arc<dyn FileSystem>, prefix: impl Into<String>) -> Self {
        Self {
            backing,
            prefix: prefix.into(),
            rights: Rights::read_only(),
            services: Vec::new(),
        }
    }

    /// Opts the job subtree into read-write access (the non-default case).
    ///
    /// Use this only when the remote job must write its outputs back into the
    /// caller's subtree; the default keeps the export read-only.
    #[must_use]
    pub fn writable(mut self) -> Self {
        self.rights = Rights::read_write();
        self
    }

    /// Adds an explicitly granted service (e.g. `#kv`) to the export scope.
    #[must_use]
    pub fn grant(mut self, service: GrantedService) -> Self {
        self.services.push(service);
        self
    }

    /// Materializes the scope as the single root the export `P9Server` serves.
    ///
    /// The job subtree is bound at `.` as a rights-gated [`SubtreeFs`]; each
    /// granted service is bound at its mount the same way. No path outside the
    /// declared subtree and services is reachable.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when a prefix is not a valid normalized path or
    /// a bind fails.
    pub fn into_root(self) -> FsResult<Arc<dyn FileSystem>> {
        let mut namespace = Namespace::new();
        let job = SubtreeFs::new(self.backing, &self.prefix, self.rights)?;
        namespace.bind(Arc::new(job), ".", ".", BindOptions::default())?;
        for service in self.services {
            let scoped = SubtreeFs::new(service.backing, &service.prefix, service.rights)?;
            namespace.bind(
                Arc::new(scoped),
                ".",
                &service.mount,
                BindOptions::default(),
            )?;
        }
        Ok(Arc::new(namespace))
    }
}

#[cfg(test)]
mod tests {
    use wanix_fs::{MemFs, NormalizedPath, OpenOptions};

    use super::*;

    fn path(value: &str) -> NormalizedPath {
        NormalizedPath::new(value).unwrap()
    }

    #[test]
    fn job_subtree_is_read_only_by_default() {
        let backing = Arc::new(MemFs::new());
        backing.create_dir_all("work").unwrap();
        backing.write_file("work/in.txt", b"data").unwrap();
        let root = ExportScope::new(backing, "work").into_root().unwrap();

        // The in-prefix file reads through the re-rooted scope.
        let mut file = root.open(&path("in.txt"), OpenOptions::read()).unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 64];
        loop {
            let n = wanix_fs::File::read(&mut *file, &mut chunk).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(buf, b"data");

        // A write-mode open is denied by the read-only rights gate.
        assert!(
            root.open(&path("in.txt"), OpenOptions::read_write())
                .is_err()
        );
    }

    #[test]
    fn writable_scope_allows_mutation() {
        let backing = Arc::new(MemFs::new());
        backing.create_dir_all("work").unwrap();
        let root = ExportScope::new(backing, "work")
            .writable()
            .into_root()
            .unwrap();
        let mut file = root
            .open(
                &path("out.txt"),
                OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: true,
                },
            )
            .unwrap();
        assert_eq!(wanix_fs::File::write(&mut *file, b"ok").unwrap(), 2);
    }

    #[test]
    fn the_scope_cannot_reach_outside_the_job_subtree() {
        let backing = Arc::new(MemFs::new());
        backing.create_dir_all("work").unwrap();
        backing.write_file("secret.txt", b"nope").unwrap();
        let root = ExportScope::new(backing, "work").into_root().unwrap();
        // `secret.txt` lives at the backing root, outside the `work` prefix, so it
        // is simply not present in the scoped namespace.
        assert!(root.metadata(&path("secret.txt")).is_err());
    }

    #[test]
    fn a_granted_service_is_reachable_at_its_mount() {
        let backing = Arc::new(MemFs::new());
        backing.create_dir_all("work").unwrap();
        let kv = Arc::new(MemFs::new());
        kv.write_file("config", b"region=us").unwrap();
        let root = ExportScope::new(backing, "work")
            .grant(GrantedService::new("#kv", kv, ".", Rights::read_only()))
            .into_root()
            .unwrap();
        let meta = root.metadata(&path("#kv/config")).unwrap();
        assert_eq!(meta.len(), b"region=us".len() as u64);
    }
}
