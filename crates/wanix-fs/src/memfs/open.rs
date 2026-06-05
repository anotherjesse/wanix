use std::collections::BTreeMap;

use crate::{FsError, FsResult, NormalizedPath, OpenOptions};

use super::node::{DEFAULT_FILE_MODE, Node};

pub(super) fn prepare_open(
    nodes: &mut BTreeMap<NormalizedPath, Node>,
    path: &NormalizedPath,
    options: OpenOptions,
) -> FsResult<()> {
    validate_open_options(path, options)?;
    create_file_if_requested(nodes, path, options)?;
    let node = nodes.get_mut(path).ok_or(FsError::NotFound)?;
    reject_directory(node)?;
    truncate_if_requested(node, options);
    Ok(())
}

fn validate_open_options(path: &NormalizedPath, options: OpenOptions) -> FsResult<()> {
    if options.create && !options.write {
        return Err(FsError::PermissionDenied);
    }
    if options.truncate && !options.write {
        return Err(FsError::PermissionDenied);
    }
    if path.as_str() == "." && (options.write || options.create || options.truncate) {
        return Err(FsError::IsDirectory);
    }
    Ok(())
}

fn create_file_if_requested(
    nodes: &mut BTreeMap<NormalizedPath, Node>,
    path: &NormalizedPath,
    options: OpenOptions,
) -> FsResult<()> {
    if !options.create || nodes.contains_key(path) {
        return Ok(());
    }
    validate_create_parent(nodes, path)?;
    nodes.insert(path.clone(), Node::file(Vec::new(), DEFAULT_FILE_MODE));
    Ok(())
}

fn validate_create_parent(
    nodes: &BTreeMap<NormalizedPath, Node>,
    path: &NormalizedPath,
) -> FsResult<()> {
    let parent = path.parent().ok_or(FsError::IsDirectory)?;
    match nodes.get(&parent) {
        Some(node) if node.is_directory() => Ok(()),
        Some(_) => Err(FsError::NotDirectory),
        None => Err(FsError::NotFound),
    }
}

fn reject_directory(node: &Node) -> FsResult<()> {
    if node.is_directory() {
        return Err(FsError::IsDirectory);
    }
    Ok(())
}

fn truncate_if_requested(node: &mut Node, options: OpenOptions) {
    if options.truncate {
        node.data.clear();
    }
}
