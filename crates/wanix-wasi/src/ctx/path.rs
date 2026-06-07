use wanix_fs::{FsError, NormalizedPath};

use crate::Errno;

const MAX_WASI_PATH_BYTES: usize = 4096;

// Namespace-rooted service devices (Plan 9 `#name` bindings). A path whose first
// component is one of these resolves from the namespace root rather than being
// joined to the working directory, so any WASI task can reach a bound service
// device by its `#name` regardless of its cwd. An unrecognized `#name` (e.g. a
// file literally named `#foo` in the cwd) stays cwd-relative.
const ROOTED_SERVICE_DEVICES: &[&str] = &[
    "#task", "#term", "#kv", "#pipe", "#plumb", "#cas", "#agent", "#mesh", "#cpu",
];

pub(super) fn is_rooted_service_path(path: &NormalizedPath) -> bool {
    let first = path.as_str().split('/').next().unwrap_or_default();
    ROOTED_SERVICE_DEVICES.contains(&first)
}

pub(super) fn wasi_path(path: &str) -> Result<NormalizedPath, Errno> {
    if path.len() > MAX_WASI_PATH_BYTES {
        return Err(Errno::Nametoolong);
    }
    if path == "." {
        return NormalizedPath::new(path).map_err(Errno::from);
    }
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|component| component == "." || component == "..")
    {
        return Err(Errno::Notcapable);
    }
    NormalizedPath::new(path).map_err(Errno::from)
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
