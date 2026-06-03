use super::config::{MAX_VIRTUAL_FILE_PATH_BYTES, validate_virtual_path_components};
use super::guest_memory::{guest_len, guest_offset_at, guest_range};
use super::{
    ERRNO_BADF, ERRNO_INVAL, ERRNO_NAMETOOLONG, ERRNO_NOENT, ERRNO_NOSYS, ERRNO_NOTCAPABLE,
    ERRNO_SUCCESS, HostState, caller_memory, wasi_stdio_fd,
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
const WASI_U32_SIZE: usize = 4;
const WASI_IOV_SIZE: usize = 2 * WASI_U32_SIZE;
const WASI_IOV_LEN_OFFSET: usize = WASI_U32_SIZE;

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
const RIGHT_PATH_FILESTAT_GET: u64 = 1 << 18;
const RIGHT_FD_FILESTAT_GET: u64 = 1 << 21;
const ALLOWED_FILE_RIGHTS: u64 =
    RIGHT_FD_READ | RIGHT_FD_SEEK | RIGHT_FD_TELL | RIGHT_FD_FILESTAT_GET;
const PREOPEN_ROOT_RIGHTS: u64 = RIGHT_PATH_OPEN | RIGHT_PATH_FILESTAT_GET | RIGHT_FD_FILESTAT_GET;

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
    linker.func_wrap("wasi_snapshot_preview1", "fd_read", fd_read)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_seek", fd_seek)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_close", fd_close)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_fdstat_get", fd_fdstat_get)?;
    linker.func_wrap("wasi_snapshot_preview1", "fd_filestat_get", fd_filestat_get)?;
    linker.func_wrap(
        "wasi_snapshot_preview1",
        "path_filestat_get",
        path_filestat_get,
    )?;
    Ok(())
}

fn fd_prestat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    prestat_ptr: i32,
) -> wasmtime::Result<i32> {
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
    write_filestat(&memory, &mut caller, stat_ptr, filetype, size)?;
    Ok(ERRNO_SUCCESS)
}

fn path_filestat_get(
    mut caller: Caller<'_, HostState>,
    dirfd: i32,
    flags: i32,
    path_ptr: i32,
    path_len: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
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

    write_filestat(&memory, &mut caller, stat_ptr, filetype, size)?;
    Ok(ERRNO_SUCCESS)
}

fn fd_read(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    iovs_ptr: i32,
    iovs_len: i32,
    nread_ptr: i32,
) -> wasmtime::Result<i32> {
    let (bytes, offset, rights_base) = match caller.data().virtual_file(fd) {
        Some(file) => (Arc::clone(&file.bytes), file.offset, file.rights_base),
        None => return Ok(ERRNO_BADF),
    };
    if rights_base & RIGHT_FD_READ == 0 {
        return Ok(ERRNO_NOTCAPABLE);
    }

    let memory = caller_memory(&caller)?;
    let iovs_len = guest_len(iovs_len)?;
    guest_range(&memory, &caller, guest_offset(nread_ptr), WASI_U32_SIZE)?;
    preflight_fd_read_iovs(&memory, &caller, iovs_ptr, iovs_len)?;

    let mut total_read = 0u32;
    let mut file_offset = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(bytes.len());
    for index in 0..iovs_len {
        let iov = read_valid_iov(&memory, &caller, iovs_ptr, index)?;
        if file_offset == bytes.len() || iov.len == 0 {
            continue;
        }
        let chunk_len = iov.len.min(bytes.len() - file_offset);
        memory.write(
            &mut caller,
            iov.ptr,
            &bytes[file_offset..file_offset + chunk_len],
        )?;
        file_offset += chunk_len;
        total_read = checked_wasi_size_add(
            total_read,
            u32::try_from(chunk_len)
                .map_err(|_| wasmtime::Error::msg("fd_read byte count exceeds u32"))?,
        )?;
    }

    memory.write(
        &mut caller,
        guest_offset(nread_ptr),
        &total_read.to_le_bytes(),
    )?;
    if let Some(file) = caller.data_mut().virtual_file_mut(fd) {
        file.offset = offset.saturating_add(u64::from(total_read));
    }
    Ok(ERRNO_SUCCESS)
}

fn fd_seek(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    offset: i64,
    whence: i32,
    result_ptr: i32,
) -> wasmtime::Result<i32> {
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

fn fd_close(mut caller: Caller<'_, HostState>, fd: i32) -> i32 {
    if caller.data_mut().close_virtual_file(fd) {
        ERRNO_SUCCESS
    } else if fd == 1 || fd == 2 || caller.data().is_virtual_preopen_fd(fd) {
        ERRNO_NOSYS
    } else {
        ERRNO_BADF
    }
}

fn fd_fdstat_get(
    mut caller: Caller<'_, HostState>,
    fd: i32,
    stat_ptr: i32,
) -> wasmtime::Result<i32> {
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

#[derive(Debug)]
struct GuestIov {
    ptr: usize,
    len: usize,
    len_u32: u32,
}

fn preflight_fd_read_iovs(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    iovs_len: usize,
) -> wasmtime::Result<()> {
    if iovs_len > 0 {
        guest_range(
            memory,
            caller,
            guest_offset_at(iovs_ptr, iovs_len - 1, WASI_IOV_SIZE)?,
            WASI_IOV_SIZE,
        )?;
    }

    let mut total_len = 0u32;
    for index in 0..iovs_len {
        let iov = read_valid_iov(memory, caller, iovs_ptr, index)?;
        total_len = checked_wasi_size_add(total_len, iov.len_u32)?;
    }
    Ok(())
}

fn read_valid_iov(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    iovs_ptr: i32,
    index: usize,
) -> wasmtime::Result<GuestIov> {
    let iov_offset = guest_offset_at(iovs_ptr, index, WASI_IOV_SIZE)?;
    let mut iov = [0u8; WASI_IOV_SIZE];
    memory.read(caller, iov_offset, &mut iov)?;

    let ptr = usize::try_from(read_iov_u32(&iov, 0))
        .map_err(|_| wasmtime::Error::msg("guest iov pointer does not fit host usize"))?;
    let len_u32 = read_iov_u32(&iov, WASI_IOV_LEN_OFFSET);
    let len = usize::try_from(len_u32)
        .map_err(|_| wasmtime::Error::msg("guest iov length does not fit host usize"))?;
    guest_range(memory, caller, ptr, len)?;
    Ok(GuestIov { ptr, len, len_u32 })
}

fn read_iov_u32(iov: &[u8; WASI_IOV_SIZE], offset: usize) -> u32 {
    let mut field = [0; WASI_U32_SIZE];
    field.copy_from_slice(&iov[offset..offset + WASI_U32_SIZE]);
    u32::from_le_bytes(field)
}

fn checked_wasi_size_add(left: u32, right: u32) -> wasmtime::Result<u32> {
    left.checked_add(right)
        .ok_or_else(|| wasmtime::Error::msg("WASI byte count overflow"))
}

fn unsupported_lookupflags(flags: i32) -> bool {
    flags.cast_unsigned() & !LOOKUPFLAGS_SYMLINK_FOLLOW != 0
}

fn write_filestat(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    stat_ptr: i32,
    filetype: u8,
    size: u64,
) -> wasmtime::Result<()> {
    let mut stat = [0u8; FILESTAT_SIZE];
    stat[FILESTAT_FILETYPE_OFFSET] = filetype;
    stat[FILESTAT_SIZE_OFFSET..FILESTAT_SIZE_OFFSET + WASI_U32_SIZE * 2]
        .copy_from_slice(&size.to_le_bytes());
    Ok(memory.write(caller, guest_offset(stat_ptr), &stat)?)
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
