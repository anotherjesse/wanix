//! Confined child namespaces: the BinVerbs trust seam.
//!
//! A resource verb (`NAME:CMD` in the shell) must run with authority over
//! exactly the resource it came from. The launcher allocates the child,
//! binds its stdio fds (resolved against the inherited namespace), then
//! writes `confine <mount-path>` to the child's `#task/<id>/ctl`: from that
//! point the child's namespace contains exactly one binding — the named
//! subtree at [`CONFINED_RESOURCE_PATH`]. No `#task`, no `#pipe`, no host
//! dirs; stdio survives because the fd table holds already-open files.
//! Widening is a future explicit act, never a default.

use std::sync::Arc;

use wanix_fs::{FsError, FsResult, NormalizedPath};
use wanix_vfs::{BindOptions, Namespace};

use super::Task;

/// Where the confining resource appears inside a confined task's namespace.
///
/// Verb code addresses its resource as `/res/...` regardless of where the
/// launcher had it mounted — the program and its authority arrive together.
pub const CONFINED_RESOURCE_PATH: &str = "res";

impl Task {
    /// Replaces this task's namespace with one containing exactly the
    /// `source` subtree of the current namespace, bound at
    /// [`CONFINED_RESOURCE_PATH`].
    ///
    /// Must happen before the task starts (a running guest must never see
    /// its namespace swapped mid-run), and after any fd binds that resolve
    /// against the wider namespace. The subtree is validated and bound
    /// outside the task state lock, so a slow backing filesystem (a mesh
    /// mount) never stalls task-table operations.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when `source` is not a valid path naming
    /// an existing subtree (the namespace root `.` is refused — confining to
    /// everything is not confinement), or when the task has already started.
    pub fn confine_to(&self, source: impl AsRef<str>) -> FsResult<()> {
        let source = NormalizedPath::new(source)?;
        if source.as_str() == "." {
            return Err(FsError::InvalidPath(
                "confine source must name a resource subtree, not the namespace root".to_owned(),
            ));
        }
        // Clone-and-swap: the ctl file's writer is the only namespace mutator
        // before start, so building the confined view from a snapshot is safe.
        let outer = self.namespace();
        let mut confined = Namespace::new();
        confined.bind(
            Arc::new(outer),
            source.as_str(),
            CONFINED_RESOURCE_PATH,
            BindOptions::default(),
        )?;
        self.write_state(|state| {
            if state.started {
                return Err(FsError::Other(
                    "cannot confine a task that already started".to_owned(),
                ));
            }
            state.namespace = confined;
            Ok(())
        })
    }
}
