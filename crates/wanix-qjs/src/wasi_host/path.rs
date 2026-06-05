use rust_wasi_quickjs::{QuickJsWasiErrno, QuickJsWasiFileStat};
use wanix_wasi::WasiFd;

use super::WanixQuickJsWasiHost;
use super::convert::{convert_errno, convert_filestat};

pub(super) fn path_filestat_get(
    host: &mut WanixQuickJsWasiHost,
    dirfd: u32,
    flags: u32,
    path: &[u8],
) -> Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
    let path = guest_path(path)?;
    host.ctx
        .path_filestat_get_with_flags(WasiFd::new(dirfd), flags, path)
        .map(convert_filestat)
        .map_err(convert_errno)
}

pub(super) fn path_filestat_set_times(
    host: &mut WanixQuickJsWasiHost,
    dirfd: u32,
    flags: u32,
    path: &[u8],
    atim: u64,
    mtim: u64,
    fstflags: u16,
) -> Result<(), QuickJsWasiErrno> {
    let path = guest_path(path)?;
    host.ctx
        .path_filestat_set_times(WasiFd::new(dirfd), flags, path, atim, mtim, fstflags)
        .map_err(convert_errno)
}

pub(super) fn path_create_directory(
    host: &mut WanixQuickJsWasiHost,
    dirfd: u32,
    path: &[u8],
) -> Result<(), QuickJsWasiErrno> {
    let path = guest_path(path)?;
    host.ctx
        .path_create_directory(WasiFd::new(dirfd), path)
        .map_err(convert_errno)
}

pub(super) fn path_readlink(
    host: &mut WanixQuickJsWasiHost,
    dirfd: u32,
    path: &[u8],
) -> Result<Vec<u8>, QuickJsWasiErrno> {
    let path = guest_path(path)?;
    host.ctx
        .path_readlink(WasiFd::new(dirfd), path)
        .map_err(convert_errno)
}

pub(super) fn path_symlink(
    host: &mut WanixQuickJsWasiHost,
    target: &[u8],
    dirfd: u32,
    path: &[u8],
) -> Result<(), QuickJsWasiErrno> {
    let path = guest_path(path)?;
    host.ctx
        .path_symlink(target, WasiFd::new(dirfd), path)
        .map_err(convert_errno)
}

pub(super) fn path_remove_directory(
    host: &mut WanixQuickJsWasiHost,
    dirfd: u32,
    path: &[u8],
) -> Result<(), QuickJsWasiErrno> {
    let path = guest_path(path)?;
    host.ctx
        .path_remove_directory(WasiFd::new(dirfd), path)
        .map_err(convert_errno)
}

pub(super) fn path_rename(
    host: &mut WanixQuickJsWasiHost,
    old_fd: u32,
    old_path: &[u8],
    new_fd: u32,
    new_path: &[u8],
) -> Result<(), QuickJsWasiErrno> {
    let old_path = guest_path(old_path)?;
    let new_path = guest_path(new_path)?;
    host.ctx
        .path_rename(WasiFd::new(old_fd), old_path, WasiFd::new(new_fd), new_path)
        .map_err(convert_errno)
}

pub(super) fn path_unlink_file(
    host: &mut WanixQuickJsWasiHost,
    dirfd: u32,
    path: &[u8],
) -> Result<(), QuickJsWasiErrno> {
    let path = guest_path(path)?;
    host.ctx
        .path_unlink_file(WasiFd::new(dirfd), path)
        .map_err(convert_errno)
}

fn guest_path(path: &[u8]) -> Result<&str, QuickJsWasiErrno> {
    std::str::from_utf8(path).map_err(|_| QuickJsWasiErrno::Inval)
}
