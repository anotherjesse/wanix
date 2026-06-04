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

#[derive(Debug, Clone, Copy)]
struct FilestatFields {
    filetype: u8,
    size: u64,
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

impl FilestatFields {
    const fn new(filetype: u8, size: u64) -> Self {
        Self {
            filetype,
            size,
            accessed_time_ns: 0,
            modified_time_ns: 0,
            changed_time_ns: 0,
        }
    }

    const fn new_with_times(
        filetype: u8,
        size: u64,
        accessed_time_ns: u64,
        modified_time_ns: u64,
        changed_time_ns: u64,
    ) -> Self {
        Self {
            filetype,
            size,
            accessed_time_ns,
            modified_time_ns,
            changed_time_ns,
        }
    }
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

fn fd_prestat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    prestat_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_prestat_get(fd))? {
        let prestat = match result {
            Ok(prestat) => prestat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        write_prestat(&memory, &mut caller, prestat_ptr, &prestat)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(fd) {
        return Ok(ERRNO_BADF);
    }
    let memory = caller_memory(&caller)?;
    let mut prestat = [0u8; PRESTAT_SIZE];
    prestat[4..8].copy_from_slice(&1_u32.to_le_bytes());
    memory.write(&mut caller, guest_offset(prestat_ptr), &prestat)?;
    Ok(ERRNO_SUCCESS)
}

fn fd_prestat_dir_name(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_prestat_get(fd))? {
        let prestat = match result {
            Ok(prestat) => prestat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let path_len = guest_len(path_len)?;
        let path = prestat.dir_name().as_bytes();
        if path_len < path.len() {
            return Ok(ERRNO_INVAL);
        }
        let memory = caller_memory(&caller)?;
        memory.write(&mut caller, guest_offset(path_ptr), path)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(fd) {
        return Ok(ERRNO_BADF);
    }
    let path_len = guest_len(path_len)?;
    if path_len < PREOPEN_ROOT_PATH.len() {
        return Ok(ERRNO_INVAL);
    }
    let memory = caller_memory(&caller)?;
    memory.write(&mut caller, guest_offset(path_ptr), PREOPEN_ROOT_PATH)?;
    Ok(ERRNO_SUCCESS)
}

#[allow(clippy::too_many_arguments)]
fn path_open(
    mut caller: Caller<'_, HostState>,
    dirfd: i32,
    dirflags: i32,
    path_ptr: i32,
    path_len: i32,
    oflags: i32,
    fs_rights_base: i64,
    fs_rights_inheriting: i64,
    fdflags: i32,
    opened_fd_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        if unsupported_lookupflags(dirflags) {
            return Ok(ERRNO_NOTCAPABLE);
        }
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let oflags = match preview1_u16_flags(oflags) {
            Ok(flags) => flags,
            Err(errno) => return Ok(errno),
        };
        let fdflags = match preview1_u16_flags(fdflags) {
            Ok(flags) => flags,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        guest_range(&memory, &caller, guest_offset(opened_fd_ptr), WASI_U32_SIZE)?;
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_open(
                dirfd,
                dirflags.cast_unsigned(),
                &path,
                oflags,
                fs_rights_base.cast_unsigned(),
                fs_rights_inheriting.cast_unsigned(),
                fdflags,
            )
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        let fd = match result {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        memory.write(&mut caller, guest_offset(opened_fd_ptr), &fd.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(dirfd) {
        return Ok(ERRNO_BADF);
    }
    if unsupported_lookupflags(dirflags) || oflags != 0 || fdflags != 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }
    let rights_base = fs_rights_base.cast_unsigned();
    let rights_inheriting = fs_rights_inheriting.cast_unsigned();
    if rights_base & !ALLOWED_FILE_RIGHTS != 0 || rights_inheriting & !ALLOWED_FILE_RIGHTS != 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let memory = caller_memory(&caller)?;
    guest_range(&memory, &caller, guest_offset(opened_fd_ptr), WASI_U32_SIZE)?;
    let path = match read_absolute_virtual_path(&memory, &caller, path_ptr, path_len)? {
        Ok(path) => path,
        Err(errno) => return Ok(errno),
    };
    if caller
        .data()
        .config()
        .read_only_virtual_file(&path)
        .is_none()
    {
        return Ok(ERRNO_NOENT);
    }

    let Some(fd) = caller.data_mut().open_virtual_file(path, rights_base) else {
        return Ok(ERRNO_INVAL);
    };
    let write = memory.write(&mut caller, guest_offset(opened_fd_ptr), &fd.to_le_bytes());
    if let Err(err) = write {
        caller.data_mut().close_virtual_file(fd);
        return Err(err.into());
    }
    Ok(ERRNO_SUCCESS)
}

fn fd_filestat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_filestat_get(fd))? {
        let stat = match result {
            Ok(stat) => stat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        write_wasi_filestat(&memory, &mut caller, stat_ptr, stat)?;
        return Ok(ERRNO_SUCCESS);
    }

    let (filetype, size) = if wasi_stdio_fd(fd).is_some() {
        (FILETYPE_CHARACTER_DEVICE, 0)
    } else if caller.data().is_virtual_preopen_fd(fd) {
        (FILETYPE_DIRECTORY, 0)
    } else if let Some(file) = caller.data().virtual_file(fd) {
        if file.rights_base & RIGHT_FD_FILESTAT_GET == 0 {
            return Ok(ERRNO_NOTCAPABLE);
        }
        let size = u64::try_from(file.bytes.len())
            .map_err(|_| wasmtime::Error::msg("virtual file length exceeds u64"))?;
        (FILETYPE_REGULAR_FILE, size)
    } else {
        return Ok(ERRNO_BADF);
    };

    let memory = caller_memory(&caller)?;
    write_filestat(
        &memory,
        &mut caller,
        stat_ptr,
        FilestatFields::new(filetype, size),
    )?;
    Ok(ERRNO_SUCCESS)
}

fn fd_filestat_set_times(
    caller: Caller<'_, HostState>,
    fd: i32,
    atim: i64,
    mtim: i64,
    fstflags: i32,
) -> wasmtime::Result<i32> {
    let fstflags = match preview1_u16_filestat_flags(fstflags) {
        Ok(flags) => flags,
        Err(errno) => return Ok(errno),
    };
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| {
        host.fd_filestat_set_times(fd, atim.cast_unsigned(), mtim.cast_unsigned(), fstflags)
    })? {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }
    if wasi_stdio_fd(fd).is_some()
        || caller.data().is_virtual_preopen_fd(fd)
        || caller.data().virtual_file(fd).is_some()
    {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

fn fd_filestat_set_size(
    caller: Caller<'_, HostState>,
    fd: i32,
    size: i64,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| {
        host.fd_filestat_set_size(fd, size.cast_unsigned())
    })? {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }
    if wasi_stdio_fd(fd).is_some()
        || caller.data().is_virtual_preopen_fd(fd)
        || caller.data().virtual_file(fd).is_some()
    {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

fn path_filestat_get(
    mut caller: Caller<'_, HostState>,
    dirfd: i32,
    flags: i32,
    path_ptr: i32,
    path_len: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        if unsupported_lookupflags(flags) {
            return Ok(ERRNO_NOTCAPABLE);
        }
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        guest_range(&memory, &caller, guest_offset(stat_ptr), FILESTAT_SIZE)?;
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_filestat_get(dirfd, flags.cast_unsigned(), &path)
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        let stat = match result {
            Ok(stat) => stat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        write_wasi_filestat(&memory, &mut caller, stat_ptr, stat)?;
        return Ok(ERRNO_SUCCESS);
    }

    if !caller.data().is_virtual_preopen_fd(dirfd) {
        return Ok(ERRNO_BADF);
    }
    if unsupported_lookupflags(flags) {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let memory = caller_memory(&caller)?;
    guest_range(&memory, &caller, guest_offset(stat_ptr), FILESTAT_SIZE)?;
    let path = match read_absolute_virtual_path(&memory, &caller, path_ptr, path_len)? {
        Ok(path) => path,
        Err(errno) => return Ok(errno),
    };
    let (filetype, size) = if let Some(bytes) = caller.data().config().read_only_virtual_file(&path)
    {
        let size = u64::try_from(bytes.len())
            .map_err(|_| wasmtime::Error::msg("virtual file length exceeds u64"))?;
        (FILETYPE_REGULAR_FILE, size)
    } else if caller
        .data()
        .config()
        .has_read_only_virtual_directory(&path)
    {
        (FILETYPE_DIRECTORY, 0)
    } else {
        return Ok(ERRNO_NOENT);
    };

    write_filestat(
        &memory,
        &mut caller,
        stat_ptr,
        FilestatFields::new(filetype, size),
    )?;
    Ok(ERRNO_SUCCESS)
}

fn path_create_directory(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) =
            with_wasi_host_u32(&caller, |host| host.path_create_directory(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

fn path_readlink(
    mut caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
    buf_ptr: i32,
    buf_len: i32,
    bufused_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let buf_len = guest_len(buf_len)?;
        let memory = caller_memory(&caller)?;
        guest_range(&memory, &caller, guest_offset(buf_ptr), buf_len)?;
        guest_range(&memory, &caller, guest_offset(bufused_ptr), WASI_U32_SIZE)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| host.path_readlink(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        let target = match result {
            Ok(target) => target,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let count = target.len().min(buf_len);
        memory.write(&mut caller, guest_offset(buf_ptr), &target[..count])?;
        let count = u32::try_from(count)
            .map_err(|_| wasmtime::Error::msg("path_readlink byte count exceeds u32"))?;
        memory.write(&mut caller, guest_offset(bufused_ptr), &count.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    let errno = unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)?;
    if errno != ERRNO_NOSYS {
        return Ok(errno);
    }
    Ok(ERRNO_NOSYS)
}

fn path_symlink(
    caller: Caller<'_, HostState>,
    old_path_ptr: i32,
    old_path_len: i32,
    dirfd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let old_path_len = match checked_wasi_path_len(old_path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let new_path_len = match checked_wasi_path_len(new_path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let old_path = read_guest_path(&memory, &caller, old_path_ptr, old_path_len)?;
        let new_path = read_guest_path(&memory, &caller, new_path_ptr, new_path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_symlink(&old_path, dirfd, &new_path)
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }

    let old_path_len = match checked_wasi_path_len(old_path_len)? {
        Ok(path_len) => path_len,
        Err(errno) => return Ok(errno),
    };
    let memory = caller_memory(&caller)?;
    read_guest_path(&memory, &caller, old_path_ptr, old_path_len)?;
    unsupported_path_mutation(&caller, dirfd, new_path_ptr, new_path_len)
}

#[allow(clippy::too_many_arguments)]
fn path_filestat_set_times(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    flags: i32,
    path_ptr: i32,
    path_len: i32,
    atim: i64,
    mtim: i64,
    fstflags: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        if unsupported_lookupflags(flags) {
            return Ok(ERRNO_NOTCAPABLE);
        }
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let fstflags = match preview1_u16_filestat_flags(fstflags) {
            Ok(flags) => flags,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_filestat_set_times(
                dirfd,
                flags.cast_unsigned(),
                &path,
                atim.cast_unsigned(),
                mtim.cast_unsigned(),
                fstflags,
            )
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }
    if unsupported_lookupflags(flags) {
        return Ok(ERRNO_NOTCAPABLE);
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

fn path_remove_directory(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) =
            with_wasi_host_u32(&caller, |host| host.path_remove_directory(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

fn path_rename(
    caller: Caller<'_, HostState>,
    old_fd: i32,
    old_path_ptr: i32,
    old_path_len: i32,
    new_fd: i32,
    new_path_ptr: i32,
    new_path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let old_fd = match preview1_fd(old_fd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let new_fd = match preview1_fd(new_fd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let old_path_len = match checked_wasi_path_len(old_path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let new_path_len = match checked_wasi_path_len(new_path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let old_path = read_guest_path(&memory, &caller, old_path_ptr, old_path_len)?;
        let new_path = read_guest_path(&memory, &caller, new_path_ptr, new_path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| {
            host.path_rename(old_fd, &old_path, new_fd, &new_path)
        })?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }

    let old_errno = unsupported_path_mutation(&caller, old_fd, old_path_ptr, old_path_len)?;
    if old_errno != ERRNO_NOSYS {
        return Ok(old_errno);
    }
    let new_errno = unsupported_path_mutation(&caller, new_fd, new_path_ptr, new_path_len)?;
    if new_errno != ERRNO_NOSYS {
        return Ok(new_errno);
    }
    Ok(ERRNO_NOSYS)
}

fn path_unlink_file(
    caller: Caller<'_, HostState>,
    dirfd: i32,
    path_ptr: i32,
    path_len: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let dirfd = match preview1_fd(dirfd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let path_len = match checked_wasi_path_len(path_len)? {
            Ok(path_len) => path_len,
            Err(errno) => return Ok(errno),
        };
        let memory = caller_memory(&caller)?;
        let path = read_guest_path(&memory, &caller, path_ptr, path_len)?;
        let Some(result) = with_wasi_host_u32(&caller, |host| host.path_unlink_file(dirfd, &path))?
        else {
            return Ok(ERRNO_BADF);
        };
        return match result {
            Ok(()) => Ok(ERRNO_SUCCESS),
            Err(errno) => Ok(errno.preview1_result()),
        };
    }
    unsupported_path_mutation(&caller, dirfd, path_ptr, path_len)
}

fn fd_readdir(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    buf_ptr: i32,
    buf_len: i32,
    cookie: i64,
    bufused_ptr: i32,
) -> wasmtime::Result<i32> {
    if caller.data().wasi_host().is_some() {
        let fd = match preview1_fd(fd) {
            Ok(fd) => fd,
            Err(errno) => return Ok(errno),
        };
        let buf_len = guest_len(buf_len)?;
        let memory = caller_memory(&caller)?;
        let buf_range = guest_range(&memory, &caller, guest_offset(buf_ptr), buf_len)?;
        guest_range(&memory, &caller, guest_offset(bufused_ptr), WASI_U32_SIZE)?;

        let Some(result) = with_wasi_host_u32(&caller, |host| host.fd_readdir(fd))? else {
            return Ok(ERRNO_BADF);
        };
        let entries = match result {
            Ok(entries) => entries,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let used = write_wasi_direntries(
            &memory,
            &mut caller,
            buf_range.start,
            buf_len,
            cookie.cast_unsigned(),
            &entries,
        )?;
        let used = u32::try_from(used)
            .map_err(|_| wasmtime::Error::msg("fd_readdir byte count exceeds u32"))?;
        memory.write(&mut caller, guest_offset(bufused_ptr), &used.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    if caller.data().is_virtual_preopen_fd(fd) {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

fn fd_seek(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    offset: i64,
    whence: i32,
    result_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| {
        let whence = QuickJsWasiWhence::from_preview1(whence)?;
        host.fd_seek(fd, offset, whence)
    })? {
        let next = match result {
            Ok(next) => next,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        memory.write(&mut caller, guest_offset(result_ptr), &next.to_le_bytes())?;
        return Ok(ERRNO_SUCCESS);
    }

    let (current, len, rights_base) = match caller.data().virtual_file(fd) {
        Some(file) => (file.offset, file.bytes.len(), file.rights_base),
        None => return Ok(ERRNO_BADF),
    };
    if rights_base & RIGHT_FD_SEEK == 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let base = match whence {
        WHENCE_SET => 0_i128,
        WHENCE_CUR => i128::from(current),
        WHENCE_END => i128::try_from(len)
            .map_err(|_| wasmtime::Error::msg("virtual file length exceeds i128"))?,
        _ => return Ok(ERRNO_INVAL),
    };
    let next = base + i128::from(offset);
    if !(0..=i128::from(u64::MAX)).contains(&next) {
        return Ok(ERRNO_INVAL);
    }
    let next = u64::try_from(next)
        .map_err(|_| wasmtime::Error::msg("validated seek offset did not fit u64"))?;

    let memory = caller_memory(&caller)?;
    memory.write(&mut caller, guest_offset(result_ptr), &next.to_le_bytes())?;
    if let Some(file) = caller.data_mut().virtual_file_mut(fd) {
        file.offset = next;
    }
    Ok(ERRNO_SUCCESS)
}

fn fd_tell(mut caller: Caller<'_, HostState>, fd: i32, result_ptr: i32) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_tell(fd))? {
        let current = match result {
            Ok(current) => current,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        memory.write(
            &mut caller,
            guest_offset(result_ptr),
            &current.to_le_bytes(),
        )?;
        return Ok(ERRNO_SUCCESS);
    }

    let (current, rights_base) = match caller.data().virtual_file(fd) {
        Some(file) => (file.offset, file.rights_base),
        None => return Ok(ERRNO_BADF),
    };
    if rights_base & RIGHT_FD_TELL == 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let memory = caller_memory(&caller)?;
    memory.write(
        &mut caller,
        guest_offset(result_ptr),
        &current.to_le_bytes(),
    )?;
    Ok(ERRNO_SUCCESS)
}

fn fd_close(mut caller: Caller<'_, HostState>, fd: i32) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_close(fd))? {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }

    if caller.data_mut().close_virtual_file(fd) {
        Ok(ERRNO_SUCCESS)
    } else if fd == 1 || fd == 2 || caller.data().is_virtual_preopen_fd(fd) {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
}

fn fd_fdstat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
    if let Some(result) = with_wasi_host(&caller, fd, |host, fd| host.fd_fdstat_get(fd))? {
        let stat = match result {
            Ok(stat) => stat,
            Err(errno) => return Ok(errno.preview1_result()),
        };
        let memory = caller_memory(&caller)?;
        write_wasi_fdstat(&memory, &mut caller, stat_ptr, stat)?;
        return Ok(ERRNO_SUCCESS);
    }

    let (filetype, rights_base, rights_inheriting) = if wasi_stdio_fd(fd).is_some() {
        (FILETYPE_CHARACTER_DEVICE, 0, 0)
    } else if caller.data().is_virtual_preopen_fd(fd) {
        (FILETYPE_DIRECTORY, PREOPEN_ROOT_RIGHTS, ALLOWED_FILE_RIGHTS)
    } else if let Some(file) = caller.data().virtual_file(fd) {
        (FILETYPE_REGULAR_FILE, file.rights_base, 0)
    } else {
        return Ok(ERRNO_BADF);
    };

    let memory = caller_memory(&caller)?;
    let mut stat = [0u8; FDSTAT_SIZE];
    stat[0] = filetype;
    stat[8..16].copy_from_slice(&rights_base.to_le_bytes());
    stat[16..24].copy_from_slice(&rights_inheriting.to_le_bytes());
    memory.write(&mut caller, guest_offset(stat_ptr), &stat)?;
    Ok(ERRNO_SUCCESS)
}

fn fd_fdstat_set_flags(
    caller: Caller<'_, HostState>,
    fd: i32,
    flags: i32,
) -> wasmtime::Result<i32> {
    if let Err(errno) = preview1_fd(fd) {
        return Ok(errno);
    }
    let flags = match preview1_u16_flags(flags) {
        Ok(flags) => flags,
        Err(errno) => return Ok(errno),
    };
    if let Some(result) =
        with_wasi_host(&caller, fd, |host, fd| host.fd_fdstat_set_flags(fd, flags))?
    {
        return Ok(match result {
            Ok(()) => ERRNO_SUCCESS,
            Err(errno) => errno.preview1_result(),
        });
    }
    if wasi_stdio_fd(fd).is_some()
        || caller.data().is_virtual_preopen_fd(fd)
        || caller.data().virtual_file(fd).is_some()
    {
        Ok(ERRNO_NOSYS)
    } else {
        Ok(ERRNO_BADF)
    }
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

fn write_filestat(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    stat_ptr: i32,
    fields: FilestatFields,
) -> wasmtime::Result<()> {
    let mut stat = [0u8; FILESTAT_SIZE];
    stat[FILESTAT_FILETYPE_OFFSET] = fields.filetype;
    stat[FILESTAT_SIZE_OFFSET..FILESTAT_SIZE_OFFSET + WASI_U32_SIZE * 2]
        .copy_from_slice(&fields.size.to_le_bytes());
    stat[FILESTAT_ATIM_OFFSET..FILESTAT_ATIM_OFFSET + WASI_U32_SIZE * 2]
        .copy_from_slice(&fields.accessed_time_ns.to_le_bytes());
    stat[FILESTAT_MTIM_OFFSET..FILESTAT_MTIM_OFFSET + WASI_U32_SIZE * 2]
        .copy_from_slice(&fields.modified_time_ns.to_le_bytes());
    stat[FILESTAT_CTIM_OFFSET..FILESTAT_CTIM_OFFSET + WASI_U32_SIZE * 2]
        .copy_from_slice(&fields.changed_time_ns.to_le_bytes());
    Ok(memory.write(caller, guest_offset(stat_ptr), &stat)?)
}

fn write_prestat(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    prestat_ptr: i32,
    prestat: &QuickJsWasiPrestat,
) -> wasmtime::Result<()> {
    let len = u32::try_from(prestat.dir_name().len())
        .map_err(|_| wasmtime::Error::msg("WASI preopen name length exceeds u32"))?;
    let mut bytes = [0u8; PRESTAT_SIZE];
    bytes[4..8].copy_from_slice(&len.to_le_bytes());
    Ok(memory.write(caller, guest_offset(prestat_ptr), &bytes)?)
}

fn write_wasi_fdstat(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    stat_ptr: i32,
    stat: QuickJsWasiFdStat,
) -> wasmtime::Result<()> {
    let mut bytes = [0u8; FDSTAT_SIZE];
    bytes[0] = stat.file_type().preview1_code();
    bytes[2..4].copy_from_slice(&stat.fdflags().to_le_bytes());
    bytes[8..16].copy_from_slice(&stat.rights_base().to_le_bytes());
    bytes[16..24].copy_from_slice(&stat.rights_inheriting().to_le_bytes());
    Ok(memory.write(caller, guest_offset(stat_ptr), &bytes)?)
}

fn write_wasi_filestat(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    stat_ptr: i32,
    stat: QuickJsWasiFileStat,
) -> wasmtime::Result<()> {
    write_filestat(
        memory,
        caller,
        stat_ptr,
        FilestatFields::new_with_times(
            stat.file_type().preview1_code(),
            stat.size(),
            stat.accessed_time_ns(),
            stat.modified_time_ns(),
            stat.changed_time_ns(),
        ),
    )
}

fn write_wasi_direntries(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    buf_ptr: usize,
    buf_len: usize,
    cookie: u64,
    entries: &[QuickJsWasiDirEntry],
) -> wasmtime::Result<usize> {
    let start = usize::try_from(cookie).unwrap_or(usize::MAX);
    if start >= entries.len() || buf_len == 0 {
        return Ok(0);
    }

    let mut used = 0usize;
    for (index, entry) in entries.iter().enumerate().skip(start) {
        let name = entry.name().as_bytes();
        let entry_len = DIRENT_SIZE
            .checked_add(name.len())
            .ok_or_else(|| wasmtime::Error::msg("WASI dirent length overflow"))?;
        let remaining = buf_len - used;
        if remaining == 0 {
            break;
        }
        let to_write = remaining.min(entry_len);
        let header = wasi_dirent_header(index, entry)?;
        let header_len = to_write.min(DIRENT_SIZE);
        memory.write(&mut *caller, buf_ptr + used, &header[..header_len])?;
        if to_write > DIRENT_SIZE {
            let name_len = to_write - DIRENT_SIZE;
            memory.write(
                &mut *caller,
                buf_ptr + used + DIRENT_SIZE,
                &name[..name_len],
            )?;
        }
        used += to_write;
        if to_write < entry_len {
            break;
        }
    }
    Ok(used)
}

fn wasi_dirent_header(
    index: usize,
    entry: &QuickJsWasiDirEntry,
) -> wasmtime::Result<[u8; DIRENT_SIZE]> {
    let next = u64::try_from(index)
        .map_err(|_| wasmtime::Error::msg("WASI dirent cookie exceeds u64"))?
        .checked_add(1)
        .ok_or_else(|| wasmtime::Error::msg("WASI dirent cookie overflow"))?;
    let name_len = u32::try_from(entry.name().len())
        .map_err(|_| wasmtime::Error::msg("WASI dirent name length exceeds u32"))?;
    let mut bytes = [0u8; DIRENT_SIZE];
    bytes[DIRENT_NEXT_OFFSET..DIRENT_NEXT_OFFSET + 8].copy_from_slice(&next.to_le_bytes());
    bytes[DIRENT_INO_OFFSET..DIRENT_INO_OFFSET + 8].copy_from_slice(&0_u64.to_le_bytes());
    bytes[DIRENT_NAMLEN_OFFSET..DIRENT_NAMLEN_OFFSET + WASI_U32_SIZE]
        .copy_from_slice(&name_len.to_le_bytes());
    bytes[DIRENT_FILETYPE_OFFSET] = entry.file_type().preview1_code();
    Ok(bytes)
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
