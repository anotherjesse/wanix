//! [`AppTree`]: the declared shape of one AppFS surface.
//!
//! The adapter is constructed with an explicit declaration of which paths the
//! guest handles (discrete ops routed over the channel) and which paths are
//! host-owned streams (per-subscription never-EOF buffers). The reserved
//! `who` presence file is always present and host-owned. V0 trees are flat:
//! every declared name is one component under the app root, matching the
//! chatroom proof tree (`post`, `stream`, `latest`, `who`, `status`).

use std::collections::BTreeSet;

use wanix_fs::{FsError, FsResult, NormalizedPath};

/// The reserved host-owned presence file name.
///
/// Reading it lists the principals currently holding open stream
/// subscriptions, one per line; the guest is never consulted.
pub const WHO_FILE: &str = "who";

/// The declared file tree of one AppFS adapter.
#[derive(Debug, Clone)]
pub struct AppTree {
    guest: BTreeSet<String>,
    streams: BTreeSet<String>,
}

/// One resolved path inside an AppFS tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppPath<'a> {
    /// The app root directory.
    Root,
    /// The host-owned `who` presence file.
    Who,
    /// A guest-handled path (discrete ops routed to the guest).
    Guest(&'a str),
    /// A host-owned stream path (never-EOF subscription buffers).
    Stream(&'a str),
}

fn check_name(name: &str, seen: &mut BTreeSet<String>) -> Result<(), String> {
    if name.is_empty() || name == "." || name.contains('/') {
        return Err(format!(
            "app tree name {name:?} must be one non-empty path component"
        ));
    }
    if name == WHO_FILE {
        return Err(format!(
            "app tree name {WHO_FILE:?} is reserved for presence"
        ));
    }
    if !seen.insert(name.to_owned()) {
        return Err(format!("app tree name {name:?} is declared twice"));
    }
    Ok(())
}

impl AppTree {
    /// Declares the tree: `guest_files` are routed to the guest, `streams`
    /// are host-owned never-EOF subscription files. `who` is implicit.
    ///
    /// # Errors
    ///
    /// Returns a message when a name is empty, contains `/`, collides with
    /// the reserved `who` file, or is declared twice.
    pub fn declare(guest_files: &[&str], streams: &[&str]) -> Result<Self, String> {
        let mut seen = BTreeSet::new();
        for name in guest_files.iter().chain(streams) {
            check_name(name, &mut seen)?;
        }
        Ok(Self {
            guest: guest_files.iter().map(|&n| n.to_owned()).collect(),
            streams: streams.iter().map(|&n| n.to_owned()).collect(),
        })
    }

    /// Whether `name` is a declared host-owned stream path.
    #[must_use]
    pub fn is_stream(&self, name: &str) -> bool {
        self.streams.contains(name)
    }

    /// The declared guest-handled names, in sorted order.
    pub(crate) fn guest_files(&self) -> impl Iterator<Item = &str> {
        self.guest.iter().map(String::as_str)
    }

    /// The declared stream names, in sorted order.
    pub(crate) fn streams(&self) -> impl Iterator<Item = &str> {
        self.streams.iter().map(String::as_str)
    }

    /// Resolves a normalized path against the declared tree.
    pub(crate) fn resolve<'a>(&self, path: &'a NormalizedPath) -> FsResult<AppPath<'a>> {
        let raw = path.as_str();
        if raw == "." {
            return Ok(AppPath::Root);
        }
        // V0 trees are flat: nested paths under a declared name do not exist.
        if raw == WHO_FILE {
            Ok(AppPath::Who)
        } else if self.guest.contains(raw) {
            Ok(AppPath::Guest(raw))
        } else if self.streams.contains(raw) {
            Ok(AppPath::Stream(raw))
        } else {
            Err(FsError::NotFound)
        }
    }
}
