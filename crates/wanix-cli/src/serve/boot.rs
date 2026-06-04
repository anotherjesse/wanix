use std::fs;
use std::path::Path;

pub(super) fn first_existing_static_route(
    static_root: &Path,
    candidates: &[&'static str],
) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|route| static_root.join(route.trim_start_matches('/')).is_file())
}

pub(super) fn first_executable_init_route(
    static_root: &Path,
    route: &'static str,
) -> Option<&'static str> {
    let path = static_root.join(route.trim_start_matches('/'));
    boot_init_is_executable(&path).then_some(route)
}

#[cfg(unix)]
fn boot_init_is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn boot_init_is_executable(path: &Path) -> bool {
    path.is_file()
}
