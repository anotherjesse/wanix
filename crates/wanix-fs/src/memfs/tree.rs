use std::collections::BTreeMap;

use crate::{FsError, FsResult, NormalizedPath};

use super::node::Node;

pub(super) fn direct_children<'a>(
    nodes: &'a BTreeMap<NormalizedPath, Node>,
    path: &'a NormalizedPath,
) -> impl Iterator<Item = &'a NormalizedPath> {
    nodes.keys().filter(move |candidate| {
        if candidate.as_str() == "." {
            return false;
        }
        candidate.parent().as_ref() == Some(path)
    })
}

pub(super) fn is_descendant_path(path: &NormalizedPath, ancestor: &NormalizedPath) -> bool {
    path.as_str()
        .strip_prefix(ancestor.as_str())
        .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn rebased_path(
    path: &NormalizedPath,
    old_root: &NormalizedPath,
    new_root: &NormalizedPath,
) -> FsResult<NormalizedPath> {
    let suffix = path
        .as_str()
        .strip_prefix(old_root.as_str())
        .ok_or_else(|| FsError::Other("memfs rename path outside moved tree".to_owned()))?;
    NormalizedPath::new(format!("{new_root}{suffix}"))
}
