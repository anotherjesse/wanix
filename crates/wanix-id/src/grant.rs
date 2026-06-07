use std::sync::{Arc, RwLock};

use wanix_fs::FileSystem;
use wanix_vfs::{Rights, SubtreeFs};

use crate::PeerId;

/// The outcome of authorizing one attach: the scoped root the peer attaches and
/// the rights enforced on it.
///
/// This is the value an [`crate::AttachPolicy`] returns; in 9P terms it is what
/// the server installs as the attaching fid's root. The root is a
/// [`SubtreeFs`], so the rights are also enforced inside the filesystem itself,
/// not only at attach time.
#[derive(Clone)]
pub struct Authorization {
    /// The filesystem the peer attaches as its root.
    pub root: Arc<dyn FileSystem>,
    /// The rights enforced on `root`.
    pub rights: Rights,
}

impl Authorization {
    /// Creates an authorization from a scoped root and its rights.
    #[must_use]
    pub fn new(root: Arc<dyn FileSystem>, rights: Rights) -> Self {
        Self { root, rights }
    }
}

/// One capability: peer `peer` may attach `aname` and receive the backing
/// filesystem re-rooted at `prefix` with `rights`.
///
/// A grant is the durable record of "this peer may bind this subtree." It is
/// matched exactly by peer identity and attach name; there is no wildcard, in
/// keeping with default-deny.
#[derive(Clone)]
pub struct Grant {
    peer: PeerId,
    aname: String,
    backing: Arc<dyn FileSystem>,
    prefix: String,
    rights: Rights,
}

impl Grant {
    /// Creates a grant for `peer` attaching `aname`, scoping `backing` to
    /// `prefix` with `rights`.
    #[must_use]
    pub fn new(
        peer: PeerId,
        aname: impl Into<String>,
        backing: Arc<dyn FileSystem>,
        prefix: impl Into<String>,
        rights: Rights,
    ) -> Self {
        Self {
            peer,
            aname: aname.into(),
            backing,
            prefix: prefix.into(),
            rights,
        }
    }

    /// Returns the peer this grant authorizes.
    #[must_use]
    pub fn peer(&self) -> PeerId {
        self.peer
    }

    /// Returns the attach name this grant matches.
    #[must_use]
    pub fn aname(&self) -> &str {
        &self.aname
    }

    /// Returns the rights this grant confers.
    #[must_use]
    pub fn rights(&self) -> Rights {
        self.rights
    }

    /// Returns whether this grant applies to `peer` attaching `aname`.
    #[must_use]
    pub fn matches(&self, peer: PeerId, aname: &str) -> bool {
        self.peer == peer && self.aname == aname
    }

    /// Materializes this grant into an [`Authorization`] with a scoped root.
    ///
    /// Returns `None` when the stored prefix is not a valid Wanix path (which a
    /// well-formed grant never is, since the constructor's callers normalize
    /// inputs, but a hand-built grant could).
    #[must_use]
    pub fn authorize(&self) -> Option<Authorization> {
        let root = SubtreeFs::new(Arc::clone(&self.backing), &self.prefix, self.rights).ok()?;
        Some(Authorization::new(Arc::new(root), self.rights))
    }
}

/// A default-deny table of [`Grant`]s, keyed by verified peer identity.
///
/// Cloning shares one underlying grant list, so an entry added through one
/// handle is visible through every clone — this is what lets a `#grant` service
/// file mutate the live table while the policy reads it. A peer with no matching
/// grant is denied; there is no implicit allow.
#[derive(Clone, Default)]
pub struct GrantTable {
    grants: Arc<RwLock<Vec<Grant>>>,
}

impl GrantTable {
    /// Creates an empty (fully default-deny) grant table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a grant to the table.
    pub fn grant(&self, grant: Grant) {
        self.grants
            .write()
            .expect("grant table lock poisoned")
            .push(grant);
    }

    /// Removes every grant matching `peer` attaching `aname`, returning how many
    /// were removed.
    pub fn revoke(&self, peer: PeerId, aname: &str) -> usize {
        let mut grants = self.grants.write().expect("grant table lock poisoned");
        let before = grants.len();
        grants.retain(|grant| !grant.matches(peer, aname));
        before - grants.len()
    }

    /// Returns the number of grants currently stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.grants.read().expect("grant table lock poisoned").len()
    }

    /// Returns whether the table holds no grants.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resolves the [`Authorization`] for `peer` attaching `aname`, or `None`
    /// when no grant matches (default-deny).
    ///
    /// The first matching grant wins, so more specific grants should be added
    /// before broader ones.
    #[must_use]
    pub fn authorize(&self, peer: PeerId, aname: &str) -> Option<Authorization> {
        let grants = self.grants.read().expect("grant table lock poisoned");
        grants
            .iter()
            .find(|grant| grant.matches(peer, aname))
            .and_then(Grant::authorize)
    }
}

#[cfg(test)]
mod tests;
