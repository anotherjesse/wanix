use super::config::{MAX_VIRTUAL_FILE_PATH_BYTES, validate_virtual_path_components};
use super::guest_memory::{guest_len, guest_range};
use super::{
    ERRNO_BADF, ERRNO_INVAL, ERRNO_NAMETOOLONG, ERRNO_NOENT, ERRNO_NOSYS, ERRNO_NOTCAPABLE,
    ERRNO_SUCCESS, HostState, caller_memory, wasi_stdio_fd,
};
use super::{
    QuickJsWasiDirEntry, QuickJsWasiErrno, QuickJsWasiFdStat, QuickJsWasiFileStat, QuickJsWasiHost,
    QuickJsWasiPrestat, QuickJsWasiWhence,
};
use crate::allocation::try_copy_bytes;
use crate::guest::guest_offset;
use std::sync::Arc;
use wasmtime::{Caller, Linker, Memory};

mod fd;
mod layout;
mod open;
mod path;
use fd::{
    fd_close, fd_fdstat_get, fd_fdstat_set_flags, fd_filestat_get, fd_filestat_set_size,
    fd_filestat_set_times, fd_prestat_dir_name, fd_prestat_get, fd_readdir, fd_seek, fd_tell,
};
use layout::{
    FilestatFields, write_filestat, write_prestat, write_wasi_direntries, write_wasi_fdstat,
    write_wasi_filestat,
};
use open::path_open;
use path::{
    path_create_directory, path_filestat_get, path_filestat_set_times, path_readlink,
    path_remove_directory, path_rename, path_symlink, path_unlink_file,
};

pub(super) const PREOPEN_ROOT_FD: i32 = 3;
pub(super) const FIRST_VIRTUAL_FILE_FD: i32 = 4;

const PREOPEN_ROOT_PATH: &[u8] = b"/";
const PRESTAT_SIZE: usize = 8;
const FDSTAT_SIZE: usize = 24;
const FILESTAT_SIZE: usize = 64;
const FILESTAT_FILETYPE_OFFSET: usize = 16;
const FILESTAT_SIZE_OFFSET: usize = 32;
const FILESTAT_ATIM_OFFSET: usize = 40;
const FILESTAT_MTIM_OFFSET: usize = 48;
const FILESTAT_CTIM_OFFSET: usize = 56;
const DIRENT_SIZE: usize = 24;
const DIRENT_NEXT_OFFSET: usize = 0;
const DIRENT_INO_OFFSET: usize = 8;
const DIRENT_NAMLEN_OFFSET: usize = 16;
const DIRENT_FILETYPE_OFFSET: usize = 20;
const WASI_U32_SIZE: usize = 4;

const FILETYPE_CHARACTER_DEVICE: u8 = 2;
const FILETYPE_DIRECTORY: u8 = 3;
const FILETYPE_REGULAR_FILE: u8 = 4;

const LOOKUPFLAGS_SYMLINK_FOLLOW: u32 = 1 << 0;

const WHENCE_SET: i32 = 0;
const WHENCE_CUR: i32 = 1;
const WHENCE_END: i32 = 2;

const RIGHT_FD_READ: u64 = 1 << 1;
const RIGHT_FD_SEEK: u64 = 1 << 2;
const RIGHT_FD_TELL: u64 = 1 << 5;
const RIGHT_PATH_OPEN: u64 = 1 << 13;
const RIGHT_PATH_READLINK: u64 = 1 << 15;
const RIGHT_PATH_FILESTAT_GET: u64 = 1 << 18;
const RIGHT_PATH_SYMLINK: u64 = 1 << 24;
const RIGHT_FD_FILESTAT_GET: u64 = 1 << 21;
const ALLOWED_FILE_RIGHTS: u64 =
    RIGHT_FD_READ | RIGHT_FD_SEEK | RIGHT_FD_TELL | RIGHT_FD_FILESTAT_GET;
const PREOPEN_ROOT_RIGHTS: u64 = RIGHT_PATH_OPEN
    | RIGHT_PATH_READLINK
    | RIGHT_PATH_FILESTAT_GET
    | RIGHT_PATH_SYMLINK
    | RIGHT_FD_FILESTAT_GET;

#[derive(Debug)]
pub(super) struct VirtualFileHandle {
    pub(super) bytes: Arc<[u8]>,
    pub(super) offset: u64,
    pub(super) rights_base: u64,
}

pub(super) fn define_imports(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("wasi_snapshot_preview1", "fd_prestat_get", fd_prestat_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_prestat_dir_name",
        fd_prestat_dir_name,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "path_open", path_open)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_readdir", fd_readdir)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_seek", fd_seek)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_tell", fd_tell)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_close", fd_close)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_fdstat_get", fd_fdstat_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_fdstat_set_flags",
        fd_fdstat_set_flags,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_filestat_get", fd_filestat_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_filestat_set_times",
        fd_filestat_set_times,
    )?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_filestat_set_size",
        fd_filestat_set_size,
    )?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_create_directory",
        path_create_directory,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "path_readlink", path_readlink)?;
    linker.func_wrap("wasi_snapshot_preview1", "path_symlink", path_symlink)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_filestat_get",
        path_filestat_get,
    )?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_filestat_set_times",
        path_filestat_set_times,
    )?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_remove_directory",
        path_remove_directory,
    )?;
    linker.func_wrap("wasi_snapshot_preview1", "path_rename", path_rename)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_unlink_file",
        path_unlink_file,
    )?;
    Ok(())
}

fn unsupported_lookupflags(flags: i32) -> bool {
    flags.cast_unsigned() & !LOOKUPFLAGS_SYMLINK_FOLLOW != 0
}

fn unsupported_path_mutation(
    caller: &Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if let Err(errno) = preview1_fd(dirfd) {
        return Ok(errno);
    }
    if let Some(result) = with_wasi_host(caller, dirfd, |host, fd| host.fd_fdstat_get(fd))? {
        if let Err(errno) = result {
            return Ok(errno.preview1_result());
        }
    } else if !caller.data().is_virtual_preopen_fd(dirfd) {
        return Ok(ERRNO_BADF);
    }
    let path_len = match checked_wasi_path_len(path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(caller)?;
    let _path = read_guest_path(&memory, caller, path_ptr, path_len)?;
    Ok(ERRNO_NOSYS)
}

fn read_absolute_virtual_path(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<Result<Vec<u8>, i32>> {
    let path_len = guest_len(path_len)?;
    if path_len
        .checked_add(1)
        .is_none_or(|len| len > MAX_VIRTUAL_FILE_PATH_BYTES)
    {
        return Ok(Err(ERRNO_NAMETOOLONG));
    }
    let path = read_guest_path(memory, caller, path_ptr, path_len)?;
    Ok(absolute_virtual_path_from_open_path(&path))
}

fn with_wasi_host<T>(
    caller: &Caller<'_, HostState>,
    fd: i32,
    call: impl FnOnce(&mut dyn QuickJsWasiHost, u32) -> Result<T, QuickJsWasiErrno>,
) -> wasmtime::Result<Option<Result<T, QuickJsWasiErrno>>> {
    let Some(host) = caller.data().wasi_host() else {
        return Ok(None);
    };
    let fd = match preview1_fd(fd) {
        Ok(fd) => fd,
        Err(errno) => return Ok(Some(Err(preview1_errno(errno)))),
    };
    let mut host = host
        .lock()
        .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
    Ok(Some(call(host.as_mut(), fd)))
}

fn with_wasi_host_u32<T>(
    caller: &Caller<'_, HostState>,
    call: impl FnOnce(&mut dyn QuickJsWasiHost) -> Result<T, QuickJsWasiErrno>,
) -> wasmtime::Result<Option<Result<T, QuickJsWasiErrno>>> {
    let Some(host) = caller.data().wasi_host() else {
        return Ok(None);
    };
    let mut host = host
        .lock()
        .map_err(|_| wasmtime::Error::msg("QuickJS WASI host lock poisoned"))?;
    Ok(Some(call(host.as_mut())))
}

fn preview1_fd(fd: i32) -> Result<u32, i32> {
    u32::try_from(fd).map_err(|_| ERRNO_BADF)
}

fn preview1_u16_flags(flags: i32) -> Result<u16, i32> {
    u16::try_from(flags).map_err(|_| ERRNO_NOTCAPABLE)
}

fn preview1_u16_filestat_flags(flags: i32) -> Result<u16, i32> {
    u16::try_from(flags).map_err(|_| ERRNO_INVAL)
}

fn preview1_errno(errno: i32) -> QuickJsWasiErrno {
    match errno {
        ERRNO_BADF => QuickJsWasiErrno::Badf,
        ERRNO_INVAL => QuickJsWasiErrno::Inval,
        ERRNO_NAMETOOLONG => QuickJsWasiErrno::Nametoolong,
        ERRNO_NOENT => QuickJsWasiErrno::Noent,
        ERRNO_NOSYS => QuickJsWasiErrno::Nosys,
        ERRNO_NOTCAPABLE => QuickJsWasiErrno::Notcapable,
        _ => QuickJsWasiErrno::Io,
    }
}

fn read_guest_path(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    path_ptr: i32,
    path_len: usize,
) -> wasmtime::Result<Vec<u8>> {
    let range = guest_range(memory, caller, guest_offset(path_ptr), path_len)?;
    try_copy_bytes(&memory.data(caller)[range], "WASI path")
        .map_err(|err| wasmtime::Error::msg(format!("{err:#}")))
}

fn checked_wasi_path_len(path_len: i32) -> wasmtime::Result<Result<usize, i32>> {
    let path_len = guest_len(path_len)?;
    if path_len > MAX_VIRTUAL_FILE_PATH_BYTES {
        return Ok(Err(ERRNO_NAMETOOLONG));
    }
    Ok(Ok(path_len))
}

fn absolute_virtual_path_from_open_path(path: &[u8]) -> Result<Vec<u8>, i32> {
    if path.starts_with(b"/") {
        return Err(ERRNO_NOTCAPABLE);
    }
    if validate_virtual_path_components(path, "WASI path").is_err() {
        return Err(ERRNO_NOTCAPABLE);
    }
    let mut absolute = Vec::new();
    absolute
        .try_reserve_exact(path.len() + 1)
        .map_err(|_| ERRNO_INVAL)?;
    absolute.push(b'/');
    absolute.extend_from_slice(path);
    Ok(absolute)
}

#[cfg(test)]
mod tests {
    use super::{
        ERRNO_BADF, ERRNO_INVAL, ERRNO_NAMETOOLONG, ERRNO_NOENT, ERRNO_NOSYS, ERRNO_NOTCAPABLE,
        QuickJsWasiErrno, preview1_errno,
    };

    #[test]
    fn preview1_errno_maps_supported_raw_codes() {
        let cases = [
            (ERRNO_BADF, QuickJsWasiErrno::Badf),
            (ERRNO_INVAL, QuickJsWasiErrno::Inval),
            (ERRNO_NAMETOOLONG, QuickJsWasiErrno::Nametoolong),
            (ERRNO_NOENT, QuickJsWasiErrno::Noent),
            (ERRNO_NOSYS, QuickJsWasiErrno::Nosys),
            (ERRNO_NOTCAPABLE, QuickJsWasiErrno::Notcapable),
        ];

        for (raw, errno) in cases {
            assert_eq!(preview1_errno(raw), errno);
        }
    }

    #[test]
    fn preview1_errno_defaults_unknown_raw_codes_to_io() {
        assert_eq!(preview1_errno(-1), QuickJsWasiErrno::Io);
        assert_eq!(preview1_errno(0), QuickJsWasiErrno::Io);
        assert_eq!(preview1_errno(i32::MAX), QuickJsWasiErrno::Io);
    }
}
