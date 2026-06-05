use std::sync::Arc;

use wanix_fs::{FileSystem, FsResult, NormalizedPath};

use super::Namespace;

/// Position used when inserting a binding at a destination path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BindPosition {
    /// Place the binding before existing bindings at the destination.
    #[default]
    First,
    /// Replace existing bindings at the destination.
    Replace,
    /// Place the binding after existing bindings at the destination.
    Last,
}

/// Options for namespace binding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindOptions {
    /// Where to insert the binding at the destination.
    pub position: BindPosition,
}

/// A public view of a stored binding.
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

#[derive(Clone)]
pub(super) struct BindTarget {
    pub(super) filesystem: Arc<dyn FileSystem>,
    pub(super) source: NormalizedPath,
}

impl Namespace {
    /// Adds a filesystem binding.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if either path is invalid, the source path
    /// does not exist, or the destination path is invalid.
    pub fn bind(
        &mut self,
        filesystem: Arc<dyn FileSystem>,
        source: impl AsRef<str>,
        destination: impl AsRef<str>,
        options: BindOptions,
    ) -> FsResult<()> {
        let source = NormalizedPath::new(source)?;
        let destination = NormalizedPath::new(destination)?;
        filesystem.metadata(&source)?;

        let target = BindTarget { filesystem, source };
        let targets = self.bindings.entry(destination).or_default();
        match options.position {
            BindPosition::First => targets.insert(0, target),
            BindPosition::Replace => {
                targets.clear();
                targets.push(target);
            }
            BindPosition::Last => targets.push(target),
        }
        Ok(())
    }

    /// Returns stored bindings in destination order, then resolution order.
    #[must_use]
    pub fn bindings(&self) -> Vec<Binding> {
        self.bindings
            .iter()
            .flat_map(|(destination, targets)| {
                targets.iter().map(|target| Binding {
                    source: target.source.clone(),
                    destination: destination.clone(),
                })
            })
            .collect()
    }

    /// Returns the number of bind targets stored in this namespace.
    #[must_use]
    pub fn binding_count(&self) -> usize {
        self.bindings.values().map(Vec::len).sum()
    }
}
