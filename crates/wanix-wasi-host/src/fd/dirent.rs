use wanix_fs::{DirEntry, FileType};
use wanix_wasi::WasiFileType;
use wasmtime::{Caller, Memory, Result};

use super::super::mem::write_bytes;

/// Byte size of a WASI Preview 1 `dirent` header.
const DIRENT_SIZE: usize = 24;
const DIRENT_NEXT_OFFSET: usize = 0;
const DIRENT_INO_OFFSET: usize = 8;
const DIRENT_NAMLEN_OFFSET: usize = 16;
const DIRENT_FILETYPE_OFFSET: usize = 20;

/// Encodes Preview 1 dirents into guest memory, returning the bytes written.
///
/// Mirrors the WASI wire encoding: a fixed [`DIRENT_SIZE`] header followed by
/// the raw name bytes per entry. A truncated entry (header or name clipped by
/// the buffer) is the final entry written, matching `fd_readdir` semantics.
pub(super) fn write_direntries<S>(
    mem: &Memory,
    caller: &mut Caller<'_, S>,
    buf: i32,
    buf_len: usize,
    cookie: usize,
    entries: &[DirEntry],
) -> Result<usize> {
    let mut used = 0usize;
    for (index, entry) in entries.iter().enumerate().skip(cookie) {
        let remaining = buf_len - used;
        if remaining == 0 {
            break;
        }
        let name = entry.name().as_bytes();
        let entry_len = DIRENT_SIZE + name.len();
        let to_write = remaining.min(entry_len);
        let header = dirent_header(index, entry);
        let header_len = to_write.min(DIRENT_SIZE);
        write_bytes(mem, caller, buf + used as i32, &header[..header_len])?;
        if to_write > DIRENT_SIZE {
            let name_len = to_write - DIRENT_SIZE;
            write_bytes(
                mem,
                caller,
                buf + (used + DIRENT_SIZE) as i32,
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

/// Maps a Wanix [`FileType`] to its WASI Preview 1 file type.
const fn wasi_file_type(file_type: FileType) -> WasiFileType {
    match file_type {
        FileType::File => WasiFileType::RegularFile,
        FileType::Directory => WasiFileType::Directory,
        FileType::Symlink => WasiFileType::SymbolicLink,
    }
}

/// Builds the fixed-size Preview 1 dirent header for `entry` at `index`.
fn dirent_header(index: usize, entry: &DirEntry) -> [u8; DIRENT_SIZE] {
    let next = (index as u64) + 1;
    let name_len = entry.name().len() as u32;
    let file_type = wasi_file_type(entry.metadata().file_type()).preview1_code();
    let mut bytes = [0u8; DIRENT_SIZE];
    bytes[DIRENT_NEXT_OFFSET..DIRENT_NEXT_OFFSET + 8].copy_from_slice(&next.to_le_bytes());
    bytes[DIRENT_INO_OFFSET..DIRENT_INO_OFFSET + 8].copy_from_slice(&0u64.to_le_bytes());
    bytes[DIRENT_NAMLEN_OFFSET..DIRENT_NAMLEN_OFFSET + 4].copy_from_slice(&name_len.to_le_bytes());
    bytes[DIRENT_FILETYPE_OFFSET] = file_type;
    bytes
}
