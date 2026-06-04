use super::{
    DIRENT_FILETYPE_OFFSET, DIRENT_INO_OFFSET, DIRENT_NAMLEN_OFFSET, DIRENT_NEXT_OFFSET,
    DIRENT_SIZE, FDSTAT_SIZE, FILESTAT_ATIM_OFFSET, FILESTAT_CTIM_OFFSET, FILESTAT_FILETYPE_OFFSET,
    FILESTAT_MTIM_OFFSET, FILESTAT_SIZE, FILESTAT_SIZE_OFFSET, HostState, PRESTAT_SIZE,
    QuickJsWasiDirEntry, QuickJsWasiFdStat, QuickJsWasiFileStat, QuickJsWasiPrestat, WASI_U32_SIZE,
};
use crate::guest::guest_offset;
use wasmtime::{Caller, Memory};

#[derive(Debug, Clone, Copy)]
pub(super) struct FilestatFields {
    filetype: u8,
    size: u64,
    accessed_time_ns: u64,
    modified_time_ns: u64,
    changed_time_ns: u64,
}

impl FilestatFields {
    pub(super) const fn new(filetype: u8, size: u64) -> Self {
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

pub(super) fn write_filestat(
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

pub(super) fn write_prestat(
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

pub(super) fn write_wasi_fdstat(
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

pub(super) fn write_wasi_filestat(
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

pub(super) fn write_wasi_direntries(
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
