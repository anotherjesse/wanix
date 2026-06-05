use rust_wasi_quickjs::{
    QuickJsWasiDirEntry, QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat,
    QuickJsWasiPrestat, QuickJsWasiWhence,
};
use wanix_wasi::{WasiFd, WasiRights};

use super::WanixQuickJsWasiHost;
use super::convert::{
    convert_errno, convert_fdstat, convert_filestat, convert_wanix_file_type, convert_whence,
};

pub(super) fn fd_prestat_get(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
) -> Result<QuickJsWasiPrestat, QuickJsWasiErrno> {
    host.ctx
        .fd_prestat_get(WasiFd::new(fd))
        .map(|prestat| QuickJsWasiPrestat::new(prestat.dir_name().to_owned()))
        .map_err(convert_errno)
}

pub(super) fn path_open(
    host: &mut WanixQuickJsWasiHost,
    dirfd: u32,
    path: &[u8],
    oflags: u16,
    rights_base: u64,
    rights_inheriting: u64,
    fdflags: u16,
) -> Result<u32, QuickJsWasiErrno> {
    let path = std::str::from_utf8(path).map_err(|_| QuickJsWasiErrno::Inval)?;
    host.ctx
        .path_open_preview1(
            WasiFd::new(dirfd),
            path,
            oflags,
            WasiRights::from_preview1_bits(rights_base),
            WasiRights::from_preview1_bits(rights_inheriting),
            fdflags,
        )
        .map(WasiFd::get)
        .map_err(convert_errno)
}

pub(super) fn fd_read(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
    buf: &mut [u8],
) -> Result<usize, QuickJsWasiErrno> {
    host.ctx
        .fd_read(WasiFd::new(fd), buf)
        .map_err(convert_errno)
}

pub(super) fn fd_read_ready(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
) -> Result<bool, QuickJsWasiErrno> {
    host.ctx
        .fd_read_ready(WasiFd::new(fd))
        .map_err(convert_errno)
}

pub(super) fn fd_readdir(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
) -> Result<Vec<QuickJsWasiDirEntry>, QuickJsWasiErrno> {
    host.ctx
        .fd_read_dir(WasiFd::new(fd))
        .map(|entries| {
            entries
                .into_iter()
                .map(|entry| {
                    QuickJsWasiDirEntry::new(
                        entry.name().to_owned(),
                        convert_wanix_file_type(entry.metadata().file_type()),
                    )
                })
                .collect()
        })
        .map_err(convert_errno)
}

pub(super) fn fd_write(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
    buf: &[u8],
) -> Result<usize, QuickJsWasiErrno> {
    host.ctx
        .fd_write(WasiFd::new(fd), buf)
        .map_err(convert_errno)
}

pub(super) fn fd_write_ready(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
) -> Result<bool, QuickJsWasiErrno> {
    host.ctx
        .fd_write_ready(WasiFd::new(fd))
        .map_err(convert_errno)
}

pub(super) fn fd_seek(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
    offset: i64,
    whence: QuickJsWasiWhence,
) -> Result<u64, QuickJsWasiErrno> {
    host.ctx
        .fd_seek(WasiFd::new(fd), offset, convert_whence(whence))
        .map_err(convert_errno)
}

pub(super) fn fd_tell(host: &mut WanixQuickJsWasiHost, fd: u32) -> Result<u64, QuickJsWasiErrno> {
    host.ctx.fd_tell(WasiFd::new(fd)).map_err(convert_errno)
}

pub(super) fn fd_close(host: &mut WanixQuickJsWasiHost, fd: u32) -> Result<(), QuickJsWasiErrno> {
    host.ctx.fd_close(WasiFd::new(fd)).map_err(convert_errno)
}

pub(super) fn fd_fdstat_get(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
) -> Result<QuickJsWasiFdStat, QuickJsWasiErrno> {
    host.ctx
        .fd_fdstat_get(WasiFd::new(fd))
        .map(convert_fdstat)
        .map_err(convert_errno)
}

pub(super) fn fd_fdstat_set_flags(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
    fdflags: u16,
) -> Result<(), QuickJsWasiErrno> {
    host.ctx
        .fd_fdstat_set_flags(WasiFd::new(fd), fdflags)
        .map_err(convert_errno)
}

pub(super) fn fd_filestat_get(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
) -> Result<QuickJsWasiFileStat, QuickJsWasiErrno> {
    host.ctx
        .fd_filestat_get(WasiFd::new(fd))
        .map(convert_filestat)
        .map_err(convert_errno)
}

pub(super) fn fd_filestat_set_times(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
    atim: u64,
    mtim: u64,
    fstflags: u16,
) -> Result<(), QuickJsWasiErrno> {
    host.ctx
        .fd_filestat_set_times(WasiFd::new(fd), atim, mtim, fstflags)
        .map_err(convert_errno)
}

pub(super) fn fd_filestat_set_size(
    host: &mut WanixQuickJsWasiHost,
    fd: u32,
    size: u64,
) -> Result<(), QuickJsWasiErrno> {
    host.ctx
        .fd_filestat_set_size(WasiFd::new(fd), size)
        .map_err(convert_errno)
}
