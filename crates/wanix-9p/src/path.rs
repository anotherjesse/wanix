use wanix_fs::NormalizedPath;

use crate::Wanix9pError;

pub(super) fn join_walk_component(
    base: &NormalizedPath,
    component: &str,
) -> Result<NormalizedPath, Wanix9pError> {
    let path = if base.as_str() == "." {
        component.to_owned()
    } else {
        format!("{}/{component}", base.as_str())
    };
    NormalizedPath::new(&path).map_err(|_| Wanix9pError::InvalidPath(path))
}

pub(super) fn rebase_path(
    path: &NormalizedPath,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
) -> Option<NormalizedPath> {
    if path == old_path {
        return Some(new_path.clone());
    }
    let suffix = descendant_suffix(path, old_path)?;
    let rebased = if new_path.as_str() == "." {
        suffix.to_owned()
    } else {
        format!("{}/{}", new_path.as_str(), suffix)
    };
    NormalizedPath::new(&rebased).ok()
}

pub(super) fn is_same_or_descendant_path(path: &NormalizedPath, base: &NormalizedPath) -> bool {
    path == base || descendant_suffix(path, base).is_some()
}

fn descendant_suffix<'a>(path: &'a NormalizedPath, base: &NormalizedPath) -> Option<&'a str> {
    let base = base.as_str();
    if base == "." {
        return Some(path.as_str());
    }
    path.as_str()
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|suffix| !suffix.is_empty())
}
