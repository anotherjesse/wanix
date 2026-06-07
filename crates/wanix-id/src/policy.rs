use crate::{Authorization, GrantTable, PeerId};

/// Decides what (if anything) a verified peer may attach.
///
/// The policy is a pure function of the verified peer identity and the
/// requested attach name. Returning `None` denies the attach; a server that
/// gets `None` should reply with a permission error and bind nothing. This
/// keeps the trust boundary — "who gets which root" — out of the wire code and
/// in one auditable place.
pub trait AttachPolicy: Send + Sync {
    /// Evaluates an attach by `peer` for `aname`.
    ///
    /// Returns the scoped [`Authorization`] to install, or `None` to deny.
    fn evaluate(&self, peer: PeerId, aname: &str) -> Option<Authorization>;
}

/// An [`AttachPolicy`] backed by a default-deny [`GrantTable`].
///
/// Because [`GrantTable`] is cheaply cloneable and shares its backing store,
/// the same table can be edited live (e.g. through a `#grant` service file)
/// while this policy reads it on each attach.
#[derive(Clone, Default)]
pub struct GrantTablePolicy {
    grants: GrantTable,
}

impl GrantTablePolicy {
    /// Creates a policy over `grants`.
    #[must_use]
    pub fn new(grants: GrantTable) -> Self {
        Self { grants }
    }

    /// Returns a handle to the underlying grant table for live edits.
    #[must_use]
    pub fn grants(&self) -> &GrantTable {
        &self.grants
    }
}

impl AttachPolicy for GrantTablePolicy {
    fn evaluate(&self, peer: PeerId, aname: &str) -> Option<Authorization> {
        self.grants.authorize(peer, aname)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wanix_fs::{FileSystem, MemFs};
    use wanix_vfs::Rights;

    use super::{AttachPolicy, GrantTablePolicy};
    use crate::{Grant, GrantTable, PeerId};

    fn backing() -> Arc<dyn FileSystem> {
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("projects/foo").unwrap();
        fs
    }

    #[test]
    fn default_deny_when_no_grant_matches() {
        let policy = GrantTablePolicy::new(GrantTable::new());
        assert!(
            policy
                .evaluate(PeerId::from_bytes([1u8; 32]), ".")
                .is_none()
        );
    }

    #[test]
    fn grant_table_policy_evaluates_through_the_table() {
        let peer = PeerId::from_bytes([9u8; 32]);
        let grants = GrantTable::new();
        grants.grant(Grant::new(
            peer,
            "projects/foo",
            backing(),
            "projects/foo",
            Rights::read_write(),
        ));
        let policy = GrantTablePolicy::new(grants);

        let authorization = policy.evaluate(peer, "projects/foo").unwrap();
        assert_eq!(authorization.rights, Rights::read_write());

        // A different attach name, or a different peer, is denied.
        assert!(policy.evaluate(peer, "docs").is_none());
        assert!(
            policy
                .evaluate(PeerId::from_bytes([8u8; 32]), "projects/foo")
                .is_none()
        );
    }
}
