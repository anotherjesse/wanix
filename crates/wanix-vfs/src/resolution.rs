use std::cmp::Reverse;
use std::sync::Arc;

use wanix_fs::{FsResult, Metadata, MetadataLookup, NormalizedPath};

use super::{BindTarget, Namespace};
use crate::path::{ResolvedTarget, immediate_child_name, join_paths, relative_to_destination};

impl Namespace {
    pub(super) fn resolve_candidates(
        &self,
        path: &NormalizedPath,
    ) -> FsResult<Vec<ResolvedTarget>> {
        let mut candidates = Vec::new();
        for (destination, targets) in &self.bindings {
            let Some(relative) = relative_to_destination(path, destination) else {
                continue;
            };
            for target in targets {
                candidates.push(ResolvedTarget {
                    filesystem: Arc::clone(&target.filesystem),
                    path: join_paths(&target.source, relative)?,
                    destination_len: destination.as_str().len(),
                });
            }
        }
        candidates.sort_by_key(|candidate| Reverse(candidate.destination_len));
        Ok(candidates)
    }

    pub(super) fn has_synthetic_children(&self, path: &NormalizedPath) -> bool {
        self.bindings
            .keys()
            .any(|destination| immediate_child_name(destination, path).is_some())
    }

    pub(super) fn synthetic_child_metadata(targets: &[BindTarget]) -> Option<Metadata> {
        Self::synthetic_child_metadata_with_lookup(targets, MetadataLookup::FollowSymlink)
    }

    fn synthetic_child_metadata_with_lookup(
        targets: &[BindTarget],
        lookup: MetadataLookup,
    ) -> Option<Metadata> {
        targets.iter().find_map(|target| {
            target
                .filesystem
                .metadata_with_lookup(&target.source, lookup)
                .ok()
        })
    }
}
