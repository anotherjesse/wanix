use std::sync::Arc;

use wanix_fs::{FileSystem, FsResult, NormalizedPath};

#[derive(Clone)]
pub(super) struct ResolvedTarget {
    pub(super) filesystem: Arc<dyn FileSystem>,
    pub(super) path: NormalizedPath,
    pub(super) destination_len: usize,
}

pub(super) fn relative_to_destination<'a>(
    path: &'a NormalizedPath,
    destination: &NormalizedPath,
) -> Option<&'a str> {
    if destination.as_str() == "." {
        return Some(if path.as_str() == "." {
            ""
        } else {
            path.as_str()
        });
    }
    if path == destination {
        return Some("");
    }
    path.as_str()
        .strip_prefix(destination.as_str())?
        .strip_prefix('/')
}

pub(super) fn join_paths(base: &NormalizedPath, relative: &str) -> FsResult<NormalizedPath> {
    if relative.is_empty() {
        return Ok(base.clone());
    }
    if base.as_str() == "." {
        NormalizedPath::new(relative)
    } else {
        NormalizedPath::new(format!("{base}/{relative}"))
    }
}

pub(super) fn immediate_child_name<'a>(
    destination: &'a NormalizedPath,
    parent: &NormalizedPath,
) -> Option<&'a str> {
    if destination == parent {
        return None;
    }

    let rest = if parent.as_str() == "." {
        destination.as_str()
    } else {
        destination
            .as_str()
            .strip_prefix(parent.as_str())?
            .strip_prefix('/')?
    };

    if rest.is_empty() {
        return None;
    }
    rest.split('/').next()
}

pub(super) fn is_direct_child(destination: &NormalizedPath, parent: &NormalizedPath) -> bool {
    destination.parent().as_ref() == Some(parent)
}
