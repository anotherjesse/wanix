use wanix_fs::{FileType, Metadata, NormalizedPath};
use wanix_protocol::{P9Attr, P9FsStat, P9Qid};

use crate::P9OwnerAttrs;

pub(super) const DT_DIR: u8 = 4;
pub(super) const DT_REG: u8 = 8;
const DT_LNK: u8 = 10;

pub(super) const P9_MODE_TYPE_MASK: u32 = 0o170000;
pub(super) const P9_MODE_DIR: u32 = 0o040000;
pub(super) const P9_MODE_REG: u32 = 0o100000;
pub(super) const P9_MODE_LNK: u32 = 0o120000;
pub(super) const P9_DEFAULT_BLOCK_SIZE: u64 = 65_536;
pub(super) const P9_FS_MAGIC: u32 = 0x0102_1997;
pub(super) const P9_DEFAULT_NAME_LENGTH: u32 = 255;

const P9_QID_TYPE_DIR: u8 = 0x80;
const P9_QID_TYPE_SYMLINK: u8 = 0x02;
const P9_QID_TYPE_FILE: u8 = 0;

const NANOS_PER_SECOND: u64 = 1_000_000_000;

const FNV1A_64_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_64_PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Copy)]
struct P9FileKind {
    qid_type: u8,
    dirent_type: u8,
    mode_type: u32,
}

impl P9FileKind {
    fn from_file_type(file_type: FileType) -> Self {
        match file_type {
            FileType::Directory => Self {
                qid_type: P9_QID_TYPE_DIR,
                dirent_type: DT_DIR,
                mode_type: P9_MODE_DIR,
            },
            FileType::Symlink => Self {
                qid_type: P9_QID_TYPE_SYMLINK,
                dirent_type: DT_LNK,
                mode_type: P9_MODE_LNK,
            },
            FileType::File => Self {
                qid_type: P9_QID_TYPE_FILE,
                dirent_type: DT_REG,
                mode_type: P9_MODE_REG,
            },
        }
    }
}

pub(super) fn qid_for_metadata(path: &NormalizedPath, metadata: Metadata) -> P9Qid {
    let kind = P9FileKind::from_file_type(metadata.file_type());
    P9Qid {
        qid_type: kind.qid_type,
        version: 0,
        path: fnv1a_64(path.as_str().as_bytes()),
    }
}

pub(super) fn attr_for_metadata(
    path: &NormalizedPath,
    metadata: Metadata,
    request_mask: u64,
    owner: P9OwnerAttrs,
) -> P9Attr {
    let (atime_seconds, atime_nanoseconds) = split_unix_time_ns(metadata.accessed_time_ns());
    let (mtime_seconds, mtime_nanoseconds) = split_unix_time_ns(metadata.modified_time_ns());
    let (ctime_seconds, ctime_nanoseconds) = split_unix_time_ns(metadata.changed_time_ns());
    P9Attr {
        valid: request_mask,
        qid: qid_for_metadata(path, metadata.clone()),
        mode: p9_mode_for_metadata(&metadata),
        uid: owner.uid.unwrap_or(0),
        gid: owner.gid.unwrap_or(0),
        nlink: metadata.link_count(),
        rdev: 0,
        size: metadata.len(),
        block_size: P9_DEFAULT_BLOCK_SIZE,
        blocks: metadata.len().div_ceil(P9_DEFAULT_BLOCK_SIZE),
        atime_seconds,
        atime_nanoseconds,
        mtime_seconds,
        mtime_nanoseconds,
        ctime_seconds,
        ctime_nanoseconds,
        btime_seconds: 0,
        btime_nanoseconds: 0,
        generation: 0,
        data_version: 0,
    }
}

pub(super) fn fs_stat() -> P9FsStat {
    P9FsStat {
        fs_type: P9_FS_MAGIC,
        block_size: P9_DEFAULT_BLOCK_SIZE as u32,
        blocks: 0,
        blocks_free: 0,
        blocks_available: 0,
        files: 0,
        files_free: 0,
        fsid: fnv1a_64(b"wanix-9p"),
        name_length: P9_DEFAULT_NAME_LENGTH,
    }
}

pub(super) fn dirent_type_for_metadata(metadata: &Metadata) -> u8 {
    P9FileKind::from_file_type(metadata.file_type()).dirent_type
}

fn p9_mode_for_metadata(metadata: &Metadata) -> u32 {
    let mode = metadata.mode();
    if mode & P9_MODE_TYPE_MASK != 0 {
        return mode;
    }
    mode | P9FileKind::from_file_type(metadata.file_type()).mode_type
}

fn split_unix_time_ns(value: u64) -> (u64, u64) {
    (value / NANOS_PER_SECOND, value % NANOS_PER_SECOND)
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV1A_64_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A_64_PRIME);
    }
    hash
}
