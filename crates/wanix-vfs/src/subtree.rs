use std::sync::Arc;

use wanix_fs::{
    DirEntry, File, FileSystem, FsError, FsResult, Metadata, MetadataLookup, NormalizedPath,
    OpenOptions,
};

/// Capability bits gating access through a [`SubtreeFs`].
///
/// A `SubtreeFs` consults these bits centrally: read operations require
/// [`Rights::read`], and every mutating operation (plus opening a file for
/// writing) requires [`Rights::write`]. Bits are additive; granting write does
/// not imply read unless [`Rights::read`] is also set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rights {
    /// Whether reads, lookups, and directory listings are permitted.
    pub read: bool,
    /// Whether mutations and write-mode opens are permitted.
    pub write: bool,
}

impl Rights {
    /// Read-only rights: lookups and reads allowed, every mutation denied.
    #[must_use]
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
        }
    }

    /// Read-write rights: reads and mutations both allowed.
    #[must_use]
    pub const fn read_write() -> Self {
        Self {
            read: true,
            write: true,
        }
    }

    /// No rights at all; every access is denied.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            read: false,
            write: false,
        }
    }
}

/// Access kind required by one [`SubtreeFs`] operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
}

/// A filesystem re-rooted at a subpath of a backing filesystem and gated by
/// [`Rights`].
///
/// `SubtreeFs` is the concrete form of a capability grant: a peer that is
/// granted `projects/foo` read-write attaches a `SubtreeFs` whose `.` maps to
/// `projects/foo` in the backing filesystem, with [`Rights::read_write`]. Every
/// path the peer presents is rebased under the prefix, and every method funnels
/// through one central `require` check so a missing right is always
/// [`FsError::PermissionDenied`].
///
/// Re-rooting only rewrites the path *string*, so on a symlink-following
/// backing (`LocalFs`, the future `RemoteFs`) an in-prefix symlink pointing at a
/// sibling still inside the backing root would otherwise escape the grant.
/// Every symlink-dereferencing method therefore also confines the rebased path
/// through [`FileSystem::confine_to_prefix`], which canonicalizes on a
/// host-backed filesystem and denies any target outside the prefix; the
/// combination is what keeps a walk from escaping the granted subtree. (Opaque
/// `read_link` returns the raw target bytes and is deliberately *not* followed.)
///
/// Re-rooting reuses the same prefix-join shape as [`super::Namespace::bind`]'s
/// source subpath; the genuinely new parts are the rights gate and the
/// symlink confinement.
pub struct SubtreeFs {
    backing: Arc<dyn FileSystem>,
    prefix: NormalizedPath,
    rights: Rights,
}

impl SubtreeFs {
    /// Creates a subtree view of `backing` rooted at `prefix` with `rights`.
    ///
    /// `prefix` is interpreted relative to `backing`'s root; `.` re-roots at the
    /// backing root itself. The prefix is validated as a [`NormalizedPath`], so
    /// it can never contain `..` and the subtree cannot be escaped upward.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::InvalidPath`] when `prefix` is not a valid Wanix path.
    pub fn new(
        backing: Arc<dyn FileSystem>,
        prefix: impl AsRef<str>,
        rights: Rights,
    ) -> FsResult<Self> {
        let prefix = NormalizedPath::new(prefix)?;
        Ok(Self {
            backing,
            prefix,
            rights,
        })
    }

    /// Returns the rights this subtree enforces.
    #[must_use]
    pub fn rights(&self) -> Rights {
        self.rights
    }

    /// Returns the backing-filesystem prefix this subtree is rooted at.
    #[must_use]
    pub fn prefix(&self) -> &NormalizedPath {
        &self.prefix
    }

    /// Central capability check: every method calls this before touching the
    /// backing filesystem.
    fn require(&self, access: Access) -> FsResult<()> {
        let granted = match access {
            Access::Read => self.rights.read,
            Access::Write => self.rights.write,
        };
        if granted {
            Ok(())
        } else {
            Err(FsError::PermissionDenied)
        }
    }

    /// Rebases a subtree-relative path onto the backing filesystem under the
    /// prefix.
    ///
    /// Because both the prefix and the incoming path are validated
    /// [`NormalizedPath`]s (no `..`, no absolute, no escaping components), the
    /// joined path is always inside the prefix.
    fn rebase(&self, path: &NormalizedPath) -> FsResult<NormalizedPath> {
        if self.prefix.as_str() == "." {
            return Ok(path.clone());
        }
        if path.as_str() == "." {
            return Ok(self.prefix.clone());
        }
        NormalizedPath::new(format!("{}/{}", self.prefix, path))
    }

    /// Rebases `path` and then confines it to the prefix against the backing
    /// filesystem's own symlink resolution.
    ///
    /// Re-rooting only rewrites the path string; a symlink that lives inside the
    /// granted prefix but whose target points to a sibling still inside the
    /// backing root would otherwise escape the grant on any symlink-following
    /// backing (`LocalFs`, the future `RemoteFs`). The backing's
    /// [`FileSystem::confine_to_prefix`] hook rejects such an escape with
    /// [`FsError::PermissionDenied`]; on opaque-symlink backings (`MemFs`) it is
    /// a no-op. Every path-resolving method that can follow a symlink rebases
    /// through this helper.
    fn rebase_confined(&self, path: &NormalizedPath) -> FsResult<NormalizedPath> {
        let rebased = self.rebase(path)?;
        self.backing.confine_to_prefix(&self.prefix, &rebased)?;
        Ok(rebased)
    }

    /// Maps the [`OpenOptions`] to the access kind they require.
    fn access_for_open(options: OpenOptions) -> Access {
        if options.write || options.create || options.truncate {
            Access::Write
        } else {
            Access::Read
        }
    }
}

impl FileSystem for SubtreeFs {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        self.require(Self::access_for_open(options))?;
        self.backing.open(&self.rebase_confined(path)?, options)
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        self.require(Access::Read)?;
        self.backing.metadata(&self.rebase_confined(path)?)
    }

    fn metadata_with_lookup(
        &self,
        path: &NormalizedPath,
        lookup: MetadataLookup,
    ) -> FsResult<Metadata> {
        self.require(Access::Read)?;
        // A `NoFollow` stat reports the link's own (opaque) metadata and never
        // dereferences the final target, so it cannot leak out-of-prefix
        // content; only a following lookup must be confined.
        let rebased = if lookup.follow_symlinks() {
            self.rebase_confined(path)?
        } else {
            self.rebase(path)?
        };
        self.backing.metadata_with_lookup(&rebased, lookup)
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        self.require(Access::Read)?;
        self.backing.read_dir(&self.rebase_confined(path)?)
    }

    fn read_link(&self, path: &NormalizedPath) -> FsResult<Vec<u8>> {
        self.require(Access::Read)?;
        self.backing.read_link(&self.rebase(path)?)
    }

    fn symlink(&self, target: &[u8], path: &NormalizedPath) -> FsResult<()> {
        self.require(Access::Write)?;
        self.backing.symlink(target, &self.rebase(path)?)
    }

    fn hard_link(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        self.require(Access::Write)?;
        // The link source is dereferenced, so it must stay inside the prefix; a
        // confined writable grant must not be able to mint a second name for an
        // out-of-prefix inode reached through an in-prefix symlink.
        self.backing
            .hard_link(&self.rebase_confined(old_path)?, &self.rebase(new_path)?)
    }

    fn create_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.require(Access::Write)?;
        self.backing.create_dir(&self.rebase(path)?)
    }

    fn remove_file(&self, path: &NormalizedPath) -> FsResult<()> {
        self.require(Access::Write)?;
        self.backing.remove_file(&self.rebase(path)?)
    }

    fn remove_dir(&self, path: &NormalizedPath) -> FsResult<()> {
        self.require(Access::Write)?;
        self.backing.remove_dir(&self.rebase(path)?)
    }

    fn rename(&self, old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
        self.require(Access::Write)?;
        self.backing
            .rename(&self.rebase(old_path)?, &self.rebase(new_path)?)
    }

    fn set_permissions(&self, path: &NormalizedPath, permissions: u32) -> FsResult<()> {
        self.require(Access::Write)?;
        self.backing
            .set_permissions(&self.rebase(path)?, permissions)
    }

    fn set_times(
        &self,
        path: &NormalizedPath,
        accessed_time_ns: u64,
        modified_time_ns: u64,
    ) -> FsResult<()> {
        self.require(Access::Write)?;
        self.backing
            .set_times(&self.rebase(path)?, accessed_time_ns, modified_time_ns)
    }
}

#[cfg(test)]
mod tests;
