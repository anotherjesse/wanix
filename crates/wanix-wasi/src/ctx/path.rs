use wanix_fs::{FsError, NormalizedPath};

use crate::Errno;

const MAX_WASI_PATH_BYTES: usize = 4096;

pub(super) fn is_rooted_service_path(path: &NormalizedPath) -> bool {
    matches!(path.as_str(), "#task" | "#term")
        || path.as_str().starts_with("#task/")
        || path.as_str().starts_with("#term/")
}

pub(super) fn wasi_path(path: &str) -> Result<NormalizedPath, Errno> {
    if path.len() > MAX_WASI_PATH_BYTES {
        return Err(Errno::Nametoolong);
    }
    if path == "." {
        return NormalizedPath::new(path).map_err(Errno::from);
    }
    if has_forbidden_wasi_path_shape(path) || has_forbidden_wasi_path_component(path) {
        return Err(Errno::Notcapable);
    }
    NormalizedPath::new(path).map_err(Errno::from)
}

fn has_forbidden_wasi_path_shape(path: &str) -> bool {
    path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || path.contains('\\')
        || path.contains('\0')
}

fn has_forbidden_wasi_path_component(path: &str) -> bool {
    path.split('/')
        .any(|component| component == "." || component == "..")
}

pub(super) fn wasi_symlink_target(target: &[u8]) -> Result<&[u8], Errno> {
    if target.len() > MAX_WASI_PATH_BYTES {
        return Err(Errno::Nametoolong);
    }
    if target.contains(&0) {
        return Err(Errno::Inval);
    }
    Ok(target)
}

pub(super) fn join_paths(
    base: &NormalizedPath,
    path: &NormalizedPath,
) -> Result<NormalizedPath, FsError> {
    if path.as_str() == "." {
        return Ok(base.clone());
    }
    if base.as_str() == "." {
        return Ok(path.clone());
    }
    NormalizedPath::new(format!("{base}/{path}"))
}
