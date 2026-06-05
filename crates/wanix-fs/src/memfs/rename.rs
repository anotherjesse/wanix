use std::collections::BTreeMap;

use crate::{FileType, FsError, FsResult, NormalizedPath};

use super::{
    node::Node,
    tree::{direct_children, is_descendant_path, rebased_path},
};

pub(super) fn rename_node_tree(
    nodes: &mut BTreeMap<NormalizedPath, Node>,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
) -> FsResult<()> {
    reject_root_rename(old_path, new_path)?;
    let old_node = nodes.get(old_path).cloned().ok_or(FsError::NotFound)?;
    if old_path == new_path {
        return Ok(());
    }

    validate_descendant_move(&old_node, old_path, new_path)?;
    validate_destination_parent(nodes, new_path)?;
    validate_replacement(nodes, &old_node, new_path)?;
    nodes.remove(new_path);

    if old_node.is_directory() {
        move_directory_tree(nodes, old_path, new_path)
    } else {
        move_file_node(nodes, old_path, new_path, old_node);
        Ok(())
    }
}

fn reject_root_rename(old_path: &NormalizedPath, new_path: &NormalizedPath) -> FsResult<()> {
    if old_path.as_str() == "." || new_path.as_str() == "." {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

fn validate_descendant_move(
    old_node: &Node,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
) -> FsResult<()> {
    if old_node.is_directory() && is_descendant_path(new_path, old_path) {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

fn validate_destination_parent(
    nodes: &BTreeMap<NormalizedPath, Node>,
    new_path: &NormalizedPath,
) -> FsResult<()> {
    let new_parent = new_path.parent().ok_or(FsError::PermissionDenied)?;
    match nodes.get(&new_parent) {
        Some(node) if node.is_directory() => Ok(()),
        Some(_) => Err(FsError::NotDirectory),
        None => Err(FsError::NotFound),
    }
}

fn validate_replacement(
    nodes: &BTreeMap<NormalizedPath, Node>,
    old_node: &Node,
    new_path: &NormalizedPath,
) -> FsResult<()> {
    let Some(new_node) = nodes.get(new_path) else {
        return Ok(());
    };
    match (old_node.kind, new_node.kind) {
        (FileType::Directory, FileType::Directory) if has_children(nodes, new_path) => {
            Err(FsError::NotEmpty)
        }
        (FileType::Directory, FileType::Directory) => Ok(()),
        (FileType::Directory, _) => Err(FsError::NotDirectory),
        (_, FileType::Directory) => Err(FsError::IsDirectory),
        _ => Ok(()),
    }
}

fn move_directory_tree(
    nodes: &mut BTreeMap<NormalizedPath, Node>,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
) -> FsResult<()> {
    let moved = moved_directory_nodes(nodes, old_path, new_path)?;
    for (old, _, _) in &moved {
        nodes.remove(old);
    }
    for (_, new, node) in moved {
        nodes.insert(new, node);
    }
    Ok(())
}

fn moved_directory_nodes(
    nodes: &BTreeMap<NormalizedPath, Node>,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
) -> FsResult<Vec<(NormalizedPath, NormalizedPath, Node)>> {
    let mut moved = Vec::new();
    for (path, node) in nodes {
        if path == old_path || is_descendant_path(path, old_path) {
            moved.push((
                path.clone(),
                rebased_path(path, old_path, new_path)?,
                node.clone(),
            ));
        }
    }
    Ok(moved)
}

fn move_file_node(
    nodes: &mut BTreeMap<NormalizedPath, Node>,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
    old_node: Node,
) {
    nodes.remove(old_path);
    nodes.insert(new_path.clone(), old_node);
}

fn has_children(nodes: &BTreeMap<NormalizedPath, Node>, path: &NormalizedPath) -> bool {
    direct_children(nodes, path).next().is_some()
}
