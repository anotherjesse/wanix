//! The acting principal behind one mounted job-device view.

use std::fmt;

/// The transport-derived identity a job-device view acts as.
///
/// Structured and opaque — a `{kind, id}` pair, never a bare 32-byte public
/// key (ADR 0004 §Open questions, convergence note): delegation certificates
/// (an attenuated principal: key + caveats) must slot into job privacy
/// filtering and quotas as a new `kind` without rototilling either. The
/// principal always comes from the transport/attach layer (ADR 0009); it is
/// never read from a payload field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobPrincipal {
    kind: String,
    id: String,
}

impl JobPrincipal {
    /// A mesh node principal, identified by its verified node id string
    /// (the `remote_id()` of the connection that attached the view).
    pub fn node(id: impl Into<String>) -> Self {
        Self {
            kind: "node".to_owned(),
            id: id.into(),
        }
    }

    /// A local same-process principal (e.g. the CLI user or a local task).
    pub fn local(id: impl Into<String>) -> Self {
        Self {
            kind: "local".to_owned(),
            id: id.into(),
        }
    }

    /// The principal kind (`"node"`, `"local"`, future delegation kinds).
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The opaque identity string within the kind.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for JobPrincipal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.id)
    }
}
