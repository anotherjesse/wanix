//! Plan 9-style namespace binding and resolution for Rust Wanix.
//!
//! This crate owns bind targets, bind order, direct and subpath resolution,
//! per-task namespace cloning, and synthesized directory views for unioned
//! bindings.

use wanix_fs::{FsResult, NormalizedPath};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix namespace binding";

/// Position used when inserting a binding at a destination path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BindPosition {
    /// Place the binding before existing bindings at the destination.
    First,
    /// Replace existing bindings at the destination.
    Replace,
    /// Place the binding after existing bindings at the destination.
    #[default]
    Last,
}

/// Options for namespace binding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindOptions {
    /// Where to insert the binding at the destination.
    pub position: BindPosition,
}

/// A stored binding between a source path and destination path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    source: NormalizedPath,
    destination: NormalizedPath,
}

impl Binding {
    /// Returns the source path.
    #[must_use]
    pub fn source(&self) -> &NormalizedPath {
        &self.source
    }

    /// Returns the destination path.
    #[must_use]
    pub fn destination(&self) -> &NormalizedPath {
        &self.destination
    }
}

/// Early namespace skeleton used to encode bind ordering before full FS wiring.
#[derive(Debug, Clone, Default)]
pub struct Namespace {
    bindings: Vec<Binding>,
}

impl Namespace {
    /// Creates an empty namespace.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a path binding.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if either path is invalid.
    pub fn bind(
        &mut self,
        source: impl AsRef<str>,
        destination: impl AsRef<str>,
        options: BindOptions,
    ) -> FsResult<()> {
        let binding = Binding {
            source: NormalizedPath::new(source)?,
            destination: NormalizedPath::new(destination.as_ref())?,
        };
        match options.position {
            BindPosition::First => self.bindings.insert(0, binding),
            BindPosition::Replace => {
                self.bindings
                    .retain(|existing| existing.destination != binding.destination);
                self.bindings.push(binding);
            }
            BindPosition::Last => self.bindings.push(binding),
        }
        Ok(())
    }

    /// Returns the currently stored bindings in resolution order.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
}

#[cfg(test)]
mod tests {
    use super::{BindOptions, BindPosition, CRATE_PURPOSE, Namespace};

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn bind_order_is_encoded() {
        let mut ns = Namespace::new();
        ns.bind("a", "mnt", BindOptions::default()).unwrap();
        ns.bind(
            "b",
            "mnt",
            BindOptions {
                position: BindPosition::First,
            },
        )
        .unwrap();
        ns.bind("c", "other", BindOptions::default()).unwrap();

        let sources = ns
            .bindings()
            .iter()
            .map(|binding| binding.source().as_str())
            .collect::<Vec<_>>();
        assert_eq!(sources, ["b", "a", "c"]);
    }

    #[test]
    fn replace_removes_existing_destination_bindings() {
        let mut ns = Namespace::new();
        ns.bind("a", "mnt", BindOptions::default()).unwrap();
        ns.bind("b", "mnt", BindOptions::default()).unwrap();
        ns.bind(
            "c",
            "mnt",
            BindOptions {
                position: BindPosition::Replace,
            },
        )
        .unwrap();

        assert_eq!(ns.bindings().len(), 1);
        assert_eq!(ns.bindings()[0].source().as_str(), "c");
    }
}
