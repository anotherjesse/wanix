//! 9P server adapters backed by Rust Wanix filesystems.
//!
//! This crate maps typed `wanix-protocol` 9P frames onto `wanix-fs`
//! filesystems. It owns fid state and filesystem error mapping, while the
//! protocol crate remains dependency-free and wire-only.

mod transport;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use wanix_fs::{
    File, FileSeekFrom, FileSystem, FileType, FsError, Metadata, MetadataLookup, NormalizedPath,
    OpenOptions,
};
use wanix_protocol::{
    P9_TATTACH, P9_TCLUNK, P9_TGETATTR, P9_TLCREATE, P9_TLOPEN, P9_TMKDIR, P9_TREAD, P9_TREADDIR,
    P9_TREADLINK, P9_TRENAMEAT, P9_TSTATFS, P9_TSYMLINK, P9_TUNLINKAT, P9_TVERSION, P9_TWALK,
    P9_TWRITE, P9_VERSION_9P2000_L, P9Attr, P9DirEntry, P9Error, P9Frame, P9FsStat, P9Qid,
    P9Version, p9_decode_tattach, p9_decode_tclunk, p9_decode_tgetattr, p9_decode_tlcreate,
    p9_decode_tlopen, p9_decode_tmkdir, p9_decode_tread, p9_decode_treaddir, p9_decode_treadlink,
    p9_decode_trenameat, p9_decode_tstatfs, p9_decode_tsymlink, p9_decode_tunlinkat,
    p9_decode_tversion, p9_decode_twalk, p9_decode_twrite, p9_dir_entry_encoded_len, p9_rattach,
    p9_rclunk, p9_rgetattr, p9_rlcreate, p9_rlerror, p9_rlopen, p9_rmkdir, p9_rread, p9_rreaddir,
    p9_rreadlink, p9_rrenameat, p9_rstatfs, p9_rsymlink, p9_runlinkat, p9_rversion, p9_rwalk,
    p9_rwrite,
};

pub use transport::{P9TransportError, P9TransportStats};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix 9P filesystem server adapters";

/// Default maximum 9P message size accepted by the Rust Wanix server.
pub const DEFAULT_MAX_MSIZE: u32 = 131_072;

const RREAD_HEADER_LEN: u32 = 11;
const RREADDIR_HEADER_LEN: u32 = 11;
const RLOPEN_OVERHEAD: u32 = 24;

const EBADF: u32 = 9;
const EACCES: u32 = 13;
const EEXIST: u32 = 17;
const ENOTDIR: u32 = 20;
const EISDIR: u32 = 21;
const EINVAL: u32 = 22;
const ENOTEMPTY: u32 = 39;
const EOPNOTSUPP: u32 = 95;

const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_CREAT: u32 = 0o100;
const O_TRUNC: u32 = 0o1000;
const AT_REMOVEDIR: u32 = 0x200;

const DT_DIR: u8 = 4;
const DT_REG: u8 = 8;
const DT_LNK: u8 = 10;

const P9_MODE_TYPE_MASK: u32 = 0o170000;
const P9_MODE_DIR: u32 = 0o040000;
const P9_MODE_REG: u32 = 0o100000;
const P9_MODE_LNK: u32 = 0o120000;
const P9_DEFAULT_BLOCK_SIZE: u64 = 65_536;
const P9_FS_MAGIC: u32 = 0x0102_1997;
const P9_DEFAULT_NAME_LENGTH: u32 = 255;

/// Error returned when a request is too malformed to turn into a 9P reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanix9pError {
    /// The request payload failed protocol decoding.
    Protocol(P9Error),
    /// A filesystem path cannot be represented as a normalized Wanix path.
    InvalidPath(String),
}

impl fmt::Display for Wanix9pError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(f, "9P protocol error: {error}"),
            Self::InvalidPath(path) => write!(f, "invalid 9P walk path: {path}"),
        }
    }
}

impl Error for Wanix9pError {}

impl From<P9Error> for Wanix9pError {
    fn from(error: P9Error) -> Self {
        Self::Protocol(error)
    }
}

/// Small in-process 9P2000.L server backed by one Wanix filesystem root.
pub struct P9Server {
    root: Arc<dyn FileSystem>,
    fids: BTreeMap<u32, FidEntry>,
    msize: u32,
    max_msize: u32,
}

struct FidEntry {
    path: NormalizedPath,
    file: Option<Box<dyn File>>,
}

impl P9Server {
    /// Creates a server around `root` using [`DEFAULT_MAX_MSIZE`].
    #[must_use]
    pub fn new(root: Arc<dyn FileSystem>) -> Self {
        Self {
            root,
            fids: BTreeMap::new(),
            msize: DEFAULT_MAX_MSIZE,
            max_msize: DEFAULT_MAX_MSIZE,
        }
    }

    /// Handles one decoded 9P request frame and returns the response frame.
    ///
    /// Filesystem and unsupported-operation failures become `Rlerror` replies
    /// using Linux errno values. Malformed typed payloads return
    /// [`Wanix9pError`] because a caller may need to tear down the connection.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the request payload cannot be decoded.
    pub fn handle_frame(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        match frame.message_type() {
            P9_TVERSION => self.handle_version(frame),
            P9_TSTATFS => self.handle_statfs(frame),
            P9_TATTACH => self.handle_attach(frame),
            P9_TWALK => self.handle_walk(frame),
            P9_TLOPEN => self.handle_open(frame),
            P9_TLCREATE => self.handle_create(frame),
            P9_TSYMLINK => self.handle_symlink(frame),
            P9_TREADLINK => self.handle_readlink(frame),
            P9_TGETATTR => self.handle_getattr(frame),
            P9_TREADDIR => self.handle_readdir(frame),
            P9_TREAD => self.handle_read(frame),
            P9_TWRITE => self.handle_write(frame),
            P9_TMKDIR => self.handle_mkdir(frame),
            P9_TRENAMEAT => self.handle_renameat(frame),
            P9_TUNLINKAT => self.handle_unlinkat(frame),
            P9_TCLUNK => self.handle_clunk(frame),
            _ => Ok(p9_rlerror(frame.tag(), EOPNOTSUPP)),
        }
    }

    fn handle_version(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let P9Version { msize, version } = p9_decode_tversion(frame)?;
        self.fids.clear();
        self.msize = msize.min(self.max_msize);
        let version = if version == P9_VERSION_9P2000_L {
            P9_VERSION_9P2000_L
        } else {
            "unknown"
        };
        Ok(p9_rversion(frame.tag(), self.msize, version)?)
    }

    fn handle_attach(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let attach = p9_decode_tattach(frame)?;
        let path = NormalizedPath::new(".").expect("root path is valid");
        let qid = match self.qid_for_path(&path) {
            Ok(qid) => qid,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        self.fids.insert(attach.fid, FidEntry { path, file: None });
        Ok(p9_rattach(frame.tag(), qid))
    }

    fn handle_statfs(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let statfs = p9_decode_tstatfs(frame)?;
        let Some(path) = self.fids.get(&statfs.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if let Err(error) = self.root.metadata(&path) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        Ok(p9_rstatfs(frame.tag(), fs_stat()))
    }

    fn handle_walk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let walk = p9_decode_twalk(frame)?;
        let Some(source) = self.fids.get(&walk.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let mut path = source.path.clone();
        let mut qids = Vec::with_capacity(walk.names.len());
        for name in &walk.names {
            path = join_walk_component(&path, name)?;
            match self.qid_for_path(&path) {
                Ok(qid) => qids.push(qid),
                Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
            }
        }
        self.fids.insert(walk.newfid, FidEntry { path, file: None });
        Ok(p9_rwalk(frame.tag(), &qids)?)
    }

    fn handle_open(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let open = p9_decode_tlopen(frame)?;
        let Some(path) = self.fids.get(&open.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let metadata = match self.root.metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let qid = qid_for_metadata(&path, metadata.clone());
        let file = if metadata.file_type() == FileType::Directory {
            if open.flags & (O_ACCMODE | O_CREAT | O_TRUNC) != 0 {
                return Ok(p9_rlerror(frame.tag(), EISDIR));
            }
            None
        } else {
            match self.root.open(&path, open_options_from_flags(open.flags)) {
                Ok(file) => Some(file),
                Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
            }
        };
        let Some(entry) = self.fids.get_mut(&open.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        entry.file = file;
        Ok(p9_rlopen(
            frame.tag(),
            qid,
            self.msize.saturating_sub(RLOPEN_OVERHEAD),
        ))
    }

    fn handle_create(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let create = p9_decode_tlcreate(frame)?;
        let Some(dir_path) = self.fids.get(&create.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &create.name)?;
        let mut options = open_options_from_flags(create.flags);
        options.create = true;
        let file = match self.root.open(&path, options) {
            Ok(file) => file,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let metadata = match self.root.metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let qid = qid_for_metadata(&path, metadata);
        let Some(entry) = self.fids.get_mut(&create.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        entry.path = path;
        entry.file = Some(file);
        Ok(p9_rlcreate(
            frame.tag(),
            qid,
            self.msize.saturating_sub(RLOPEN_OVERHEAD),
        ))
    }

    fn handle_symlink(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let symlink = p9_decode_tsymlink(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&symlink.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &symlink.name)?;
        if let Err(error) = self.root.symlink(symlink.target.as_bytes(), &path) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let metadata = match self.metadata_no_follow(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        Ok(p9_rsymlink(frame.tag(), qid_for_metadata(&path, metadata)))
    }

    fn handle_readlink(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let readlink = p9_decode_treadlink(frame)?;
        let Some(path) = self.fids.get(&readlink.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let target = match self.root.read_link(&path) {
            Ok(target) => target,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let Ok(target) = String::from_utf8(target) else {
            return Ok(p9_rlerror(frame.tag(), EINVAL));
        };
        Ok(p9_rreadlink(frame.tag(), &target)?)
    }

    fn handle_getattr(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let getattr = p9_decode_tgetattr(frame)?;
        let Some(path) = self.fids.get(&getattr.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let metadata = match self.metadata_no_follow(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let attr = attr_for_metadata(&path, metadata, getattr.request_mask);
        Ok(p9_rgetattr(frame.tag(), &attr))
    }

    fn handle_readdir(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let read = p9_decode_treaddir(frame)?;
        let Some(path) = self.fids.get(&read.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let entries = match self.root.read_dir(&path) {
            Ok(entries) => entries,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let max_data_len = read
            .count
            .min(self.msize.saturating_sub(RREADDIR_HEADER_LEN));
        let mut data_len = 0_u32;
        let mut cursor = 0_u64;
        let mut p9_entries = Vec::new();
        for entry in entries {
            cursor += 1;
            if cursor <= read.offset {
                continue;
            }
            let child_path = join_walk_component(&path, entry.name())?;
            let p9_entry = P9DirEntry {
                qid: qid_for_metadata(&child_path, entry.metadata().clone()),
                offset: cursor,
                dirent_type: dirent_type_for_metadata(entry.metadata()),
                name: entry.name().to_owned(),
            };
            let entry_len = p9_dir_entry_encoded_len(&p9_entry)?;
            let Ok(entry_len) = u32::try_from(entry_len) else {
                break;
            };
            let Some(next_len) = data_len.checked_add(entry_len) else {
                break;
            };
            if next_len > max_data_len {
                break;
            }
            data_len = next_len;
            p9_entries.push(p9_entry);
        }
        Ok(p9_rreaddir(frame.tag(), &p9_entries)?)
    }

    fn handle_read(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let read = p9_decode_tread(frame)?;
        let Some(entry) = self.fids.get_mut(&read.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if let Err(error) = file.seek(FileSeekFrom::Start(read.offset)) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let count = read.count.min(self.msize.saturating_sub(RREAD_HEADER_LEN)) as usize;
        let mut buf = vec![0; count];
        let read_count = match file.read(&mut buf) {
            Ok(read_count) => read_count,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        buf.truncate(read_count);
        Ok(p9_rread(frame.tag(), &buf)?)
    }

    fn handle_write(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let write = p9_decode_twrite(frame)?;
        let Some(entry) = self.fids.get_mut(&write.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if let Err(error) = file.seek(FileSeekFrom::Start(write.offset)) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let count = match file.write(&write.data) {
            Ok(count) => count,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        Ok(p9_rwrite(frame.tag(), count as u32))
    }

    fn handle_mkdir(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let mkdir = p9_decode_tmkdir(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&mkdir.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &mkdir.name)?;
        if let Err(error) = self.root.create_dir(&path) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let metadata = match self.root.metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        Ok(p9_rmkdir(frame.tag(), qid_for_metadata(&path, metadata)))
    }

    fn handle_renameat(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let rename = p9_decode_trenameat(frame)?;
        let Some(old_dir_path) = self
            .fids
            .get(&rename.old_dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(new_dir_path) = self
            .fids
            .get(&rename.new_dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let old_path = join_walk_component(&old_dir_path, &rename.old_name)?;
        let new_path = join_walk_component(&new_dir_path, &rename.new_name)?;
        match self.root.rename(&old_path, &new_path) {
            Ok(()) => Ok(p9_rrenameat(frame.tag())),
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }

    fn handle_unlinkat(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let unlink = p9_decode_tunlinkat(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&unlink.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = join_walk_component(&dir_path, &unlink.name)?;
        let result = if unlink.flags & AT_REMOVEDIR != 0 {
            self.root.remove_dir(&path)
        } else {
            self.root.remove_file(&path)
        };
        match result {
            Ok(()) => Ok(p9_runlinkat(frame.tag())),
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }

    fn handle_clunk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let clunk = p9_decode_tclunk(frame)?;
        if self.fids.remove(&clunk.fid).is_some() {
            Ok(p9_rclunk(frame.tag()))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    fn qid_for_path(&self, path: &NormalizedPath) -> Result<P9Qid, FsError> {
        Ok(qid_for_metadata(path, self.metadata_no_follow(path)?))
    }

    fn metadata_no_follow(&self, path: &NormalizedPath) -> Result<Metadata, FsError> {
        self.root
            .metadata_with_lookup(path, MetadataLookup::NoFollow)
    }
}

fn join_walk_component(
    base: &NormalizedPath,
    component: &str,
) -> Result<NormalizedPath, Wanix9pError> {
    let path = if base.as_str() == "." {
        component.to_owned()
    } else {
        format!("{}/{component}", base.as_str())
    };
    NormalizedPath::new(&path).map_err(|_| Wanix9pError::InvalidPath(path))
}

fn qid_for_metadata(path: &NormalizedPath, metadata: Metadata) -> P9Qid {
    let qid_type = match metadata.file_type() {
        FileType::Directory => 0x80,
        FileType::Symlink => 0x02,
        FileType::File => 0,
    };
    P9Qid {
        qid_type,
        version: 0,
        path: fnv1a_64(path.as_str().as_bytes()),
    }
}

fn attr_for_metadata(path: &NormalizedPath, metadata: Metadata, request_mask: u64) -> P9Attr {
    let (atime_seconds, atime_nanoseconds) = split_unix_time_ns(metadata.accessed_time_ns());
    let (mtime_seconds, mtime_nanoseconds) = split_unix_time_ns(metadata.modified_time_ns());
    let (ctime_seconds, ctime_nanoseconds) = split_unix_time_ns(metadata.changed_time_ns());
    P9Attr {
        valid: request_mask,
        qid: qid_for_metadata(path, metadata.clone()),
        mode: p9_mode_for_metadata(&metadata),
        uid: 0,
        gid: 0,
        nlink: 1,
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

fn p9_mode_for_metadata(metadata: &Metadata) -> u32 {
    let mode = metadata.mode();
    if mode & P9_MODE_TYPE_MASK != 0 {
        return mode;
    }
    mode | match metadata.file_type() {
        FileType::Directory => P9_MODE_DIR,
        FileType::Symlink => P9_MODE_LNK,
        FileType::File => P9_MODE_REG,
    }
}

fn split_unix_time_ns(value: u64) -> (u64, u64) {
    (value / 1_000_000_000, value % 1_000_000_000)
}

fn fs_stat() -> P9FsStat {
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

fn dirent_type_for_metadata(metadata: &Metadata) -> u8 {
    match metadata.file_type() {
        FileType::Directory => DT_DIR,
        FileType::Symlink => DT_LNK,
        FileType::File => DT_REG,
    }
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn open_options_from_flags(flags: u32) -> OpenOptions {
    let access = flags & O_ACCMODE;
    OpenOptions {
        read: access != O_WRONLY,
        write: access == O_WRONLY || access == O_RDWR,
        create: flags & O_CREAT != 0,
        truncate: flags & O_TRUNC != 0,
    }
}

fn errno_for_fs(error: &FsError) -> u32 {
    match error {
        FsError::InvalidPath(_) | FsError::InvalidOffset | FsError::InvalidTime => EINVAL,
        FsError::NotFound => 2,
        FsError::NotSupported => EOPNOTSUPP,
        FsError::PermissionDenied => EACCES,
        FsError::AlreadyExists => EEXIST,
        FsError::NotDirectory => ENOTDIR,
        FsError::IsDirectory => EISDIR,
        FsError::InvalidFd => EBADF,
        FsError::NotEmpty => ENOTEMPTY,
        FsError::Other(_) => EINVAL,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use wanix_fs::{LocalFs, MemFs};
    use wanix_protocol::{
        P9_RATTACH, P9_RGETATTR, P9_RLCREATE, P9_RLERROR, P9_RLOPEN, P9_RMKDIR, P9_RREAD,
        P9_RREADDIR, P9_RREADLINK, P9_RRENAMEAT, P9_RSTATFS, P9_RSYMLINK, P9_RUNLINKAT,
        P9_RVERSION, P9_RWALK, P9_RWRITE, P9DirEntry, p9_decode_rgetattr, p9_decode_rlcreate,
        p9_decode_rlerror, p9_decode_rlopen, p9_decode_rmkdir, p9_decode_rread, p9_decode_rreaddir,
        p9_decode_rreadlink, p9_decode_rstatfs, p9_decode_rsymlink, p9_decode_rversion,
        p9_decode_rwalk, p9_decode_rwrite, p9_dir_entry_encoded_len, p9_tattach, p9_tclunk,
        p9_tgetattr, p9_tlcreate, p9_tlopen, p9_tmkdir, p9_tread, p9_treaddir, p9_treadlink,
        p9_trenameat, p9_tstatfs, p9_tsymlink, p9_tunlinkat, p9_tversion, p9_twalk, p9_twrite,
    };

    use super::*;

    #[test]
    fn purpose_is_declared() {
        assert!(!CRATE_PURPOSE.is_empty());
    }

    #[test]
    fn server_reads_file_through_version_attach_walk_open_read_clunk() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello 9p").unwrap();
        let mut server = server(fs);

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        assert_eq!(p9_decode_rversion(&response).unwrap().msize, 8192);

        let response = server
            .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);

        let response = server
            .handle_frame(&p9_twalk(3, 1, 2, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert_eq!(p9_decode_rwalk(&response).unwrap().len(), 1);

        let response = server.handle_frame(&p9_tlopen(4, 2, 0)).unwrap();
        assert_eq!(response.message_type(), P9_RLOPEN);
        assert_eq!(
            p9_decode_rlopen(&response).unwrap().1,
            8192 - RLOPEN_OVERHEAD
        );

        let response = server.handle_frame(&p9_tread(5, 2, 0, 5)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"hello");

        let response = server.handle_frame(&p9_tclunk(6, 2)).unwrap();
        assert_eq!(response.message_type(), wanix_protocol::P9_RCLUNK);
    }

    #[test]
    fn statfs_reports_synthetic_wanix_filesystem_stats() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello 9p").unwrap();
        let mut server = server(fs);

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        let response = server
            .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);

        let response = server.handle_frame(&p9_tstatfs(3, 1)).unwrap();
        assert_eq!(response.message_type(), P9_RSTATFS);
        let stat = p9_decode_rstatfs(&response).unwrap();
        assert_eq!(stat.fs_type, P9_FS_MAGIC);
        assert_eq!(stat.block_size, P9_DEFAULT_BLOCK_SIZE as u32);
        assert_eq!(stat.name_length, P9_DEFAULT_NAME_LENGTH);
        assert_ne!(stat.fsid, 0);
    }

    #[test]
    fn statfs_unknown_fid_returns_bad_fd() {
        let mut server = server(Arc::new(MemFs::new()));

        let response = server.handle_frame(&p9_tstatfs(3, 99)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn server_writes_file_through_open_write() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("out.txt", b"xxxxxxxx").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["out.txt"]);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(3, 2, O_RDWR | O_TRUNC))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );

        let response = server
            .handle_frame(&p9_twrite(4, 2, 0, b"made").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 4);
        assert_eq!(fs.read_file("out.txt").unwrap(), b"made");
    }

    #[test]
    fn lcreate_creates_file_and_opens_created_fid() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tlcreate(2, 1, "new.txt", O_RDWR, 0o100664, 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLCREATE);
        let (qid, iounit) = p9_decode_rlcreate(&response).unwrap();
        assert_eq!(qid.qid_type, 0);
        assert_eq!(iounit, DEFAULT_MAX_MSIZE - RLOPEN_OVERHEAD);

        let response = server
            .handle_frame(&p9_twrite(3, 1, 0, b"created over 9p").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 15);
        assert_eq!(fs.read_file("new.txt").unwrap(), b"created over 9p");
    }

    #[test]
    fn mkdir_renameat_and_unlinkat_mutate_filesystem() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("old.txt", b"moved").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tmkdir(2, 1, "made", 0o040755, 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RMKDIR);
        assert_eq!(p9_decode_rmkdir(&response).unwrap().qid_type, 0x80);
        assert_eq!(
            fs.metadata(&NormalizedPath::new("made").unwrap())
                .unwrap()
                .file_type(),
            FileType::Directory
        );

        let response = server
            .handle_frame(&p9_tmkdir(3, 1, "dest", 0o040755, 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RMKDIR);
        walk(&mut server, 1, 2, &["dest"]);

        let response = server
            .handle_frame(&p9_trenameat(4, 1, "old.txt", 2, "new.txt").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RRENAMEAT);
        assert_eq!(fs.read_file("dest/new.txt").unwrap(), b"moved");
        assert_eq!(
            fs.metadata(&NormalizedPath::new("old.txt").unwrap()),
            Err(FsError::NotFound)
        );

        let response = server
            .handle_frame(&p9_tunlinkat(5, 2, "new.txt", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RUNLINKAT);
        assert_eq!(
            fs.metadata(&NormalizedPath::new("dest/new.txt").unwrap()),
            Err(FsError::NotFound)
        );

        let response = server
            .handle_frame(&p9_tunlinkat(6, 1, "made", AT_REMOVEDIR).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RUNLINKAT);
        assert_eq!(
            fs.metadata(&NormalizedPath::new("made").unwrap()),
            Err(FsError::NotFound)
        );
    }

    #[test]
    fn unlinkat_directory_flag_controls_remove_kind() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("file.txt", b"file").unwrap();
        fs.create_dir_all("dir").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tunlinkat(2, 1, "dir", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EISDIR);

        let response = server
            .handle_frame(&p9_tunlinkat(3, 1, "file.txt", AT_REMOVEDIR).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, ENOTDIR);
    }

    #[test]
    #[cfg(unix)]
    fn symlink_and_readlink_mutate_host_backed_filesystem() {
        let root = temp_dir("wanix-9p-symlink");
        fs::write(root.join("target.txt"), b"linked data").unwrap();
        let local = Arc::new(LocalFs::new(&root).unwrap());
        let root_fs: Arc<dyn FileSystem> = local;
        let mut server = P9Server::new(root_fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tsymlink(2, 1, "link.txt", "target.txt", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RSYMLINK);
        assert_eq!(p9_decode_rsymlink(&response).unwrap().qid_type, 0x02);
        assert_eq!(
            fs::read_link(root.join("link.txt")).unwrap(),
            std::path::PathBuf::from("target.txt")
        );

        let response = server
            .handle_frame(&p9_twalk(3, 1, 2, &["link.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        let qids = p9_decode_rwalk(&response).unwrap();
        assert_eq!(qids[0].qid_type, 0x02);

        let response = server.handle_frame(&p9_tgetattr(4, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.qid.qid_type, 0x02);
        assert_eq!(attr.mode & P9_MODE_TYPE_MASK, P9_MODE_LNK);
        assert_eq!(attr.size, "target.txt".len() as u64);

        let response = server.handle_frame(&p9_treadlink(5, 2)).unwrap();
        assert_eq!(response.message_type(), P9_RREADLINK);
        assert_eq!(p9_decode_rreadlink(&response).unwrap(), "target.txt");

        let response = server.handle_frame(&p9_tlopen(6, 2, 0)).unwrap();
        assert_eq!(response.message_type(), P9_RLOPEN);
        let response = server.handle_frame(&p9_tread(7, 2, 0, 11)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"linked data");
    }

    #[test]
    #[cfg(unix)]
    fn readlink_regular_file_returns_invalid() {
        let root = temp_dir("wanix-9p-readlink-regular");
        fs::write(root.join("target.txt"), b"not a link").unwrap();
        let local = Arc::new(LocalFs::new(&root).unwrap());
        let root_fs: Arc<dyn FileSystem> = local;
        let mut server = P9Server::new(root_fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["target.txt"]);
        let response = server.handle_frame(&p9_treadlink(3, 2)).unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EINVAL);
    }

    #[test]
    fn server_lists_directory_entries_through_readdir() {
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("bin").unwrap();
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server.handle_frame(&p9_tlopen(2, 1, 0)).unwrap();
        assert_eq!(response.message_type(), P9_RLOPEN);
        let (root_qid, _) = p9_decode_rlopen(&response).unwrap();
        assert_eq!(root_qid.qid_type, 0x80);

        let response = server.handle_frame(&p9_treaddir(3, 1, 0, 4096)).unwrap();
        assert_eq!(response.message_type(), P9_RREADDIR);
        let entries = p9_decode_rreaddir(&response).unwrap();
        assert_eq!(entry_names(&entries), ["bin", "hello.txt"]);
        assert_eq!(entries[0].offset, 1);
        assert_eq!(entries[0].dirent_type, DT_DIR);
        assert_eq!(entries[0].qid.qid_type, 0x80);
        assert_eq!(entries[1].offset, 2);
        assert_eq!(entries[1].dirent_type, DT_REG);
        assert_eq!(entries[1].qid.qid_type, 0);
    }

    #[test]
    fn readdir_uses_offset_cookie_and_count_budget() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("a.txt", b"a").unwrap();
        fs.write_file("b.txt", b"b").unwrap();
        fs.write_file("c.txt", b"c").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(2, 1, 0))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );
        let one_entry_budget = p9_dir_entry_encoded_len(&P9DirEntry {
            qid: P9Qid {
                qid_type: 0,
                version: 0,
                path: 0,
            },
            offset: 0,
            dirent_type: DT_REG,
            name: "a.txt".to_owned(),
        })
        .unwrap() as u32;

        let response = server
            .handle_frame(&p9_treaddir(3, 1, 0, one_entry_budget))
            .unwrap();
        let entries = p9_decode_rreaddir(&response).unwrap();
        assert_eq!(entry_names(&entries), ["a.txt"]);
        let resume_offset = entries[0].offset;

        let response = server
            .handle_frame(&p9_treaddir(4, 1, resume_offset, 4096))
            .unwrap();
        let entries = p9_decode_rreaddir(&response).unwrap();
        assert_eq!(entry_names(&entries), ["b.txt", "c.txt"]);

        let response = server
            .handle_frame(&p9_treaddir(5, 1, 0, one_entry_budget - 1))
            .unwrap();
        assert!(p9_decode_rreaddir(&response).unwrap().is_empty());
    }

    #[test]
    fn readdir_file_path_returns_not_directory() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["hello.txt"]);
        let response = server.handle_frame(&p9_treaddir(3, 2, 0, 4096)).unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, ENOTDIR);
    }

    #[test]
    fn getattr_reports_file_metadata_before_open() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello p9").unwrap();
        fs.set_times(
            &NormalizedPath::new("hello.txt").unwrap(),
            1_234_000_000_005,
            2_345_000_000_006,
        )
        .unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["hello.txt"]);
        let response = server
            .handle_frame(&p9_tgetattr(3, 2, 0x1234_5678))
            .unwrap();

        assert_eq!(response.message_type(), P9_RGETATTR);
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.valid, 0x1234_5678);
        assert_eq!(attr.qid.qid_type, 0);
        assert_eq!(attr.mode, P9_MODE_REG | 0o644);
        assert_eq!(attr.size, 8);
        assert_eq!(attr.block_size, P9_DEFAULT_BLOCK_SIZE);
        assert_eq!(attr.blocks, 1);
        assert_eq!(attr.uid, 0);
        assert_eq!(attr.gid, 0);
        assert_eq!(attr.nlink, 1);
        assert_eq!(attr.atime_seconds, 1_234);
        assert_eq!(attr.atime_nanoseconds, 5);
        assert_eq!(attr.mtime_seconds, 2_345);
        assert_eq!(attr.mtime_nanoseconds, 6);
    }

    #[test]
    fn getattr_reports_directory_metadata() {
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("bin").unwrap();
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server.handle_frame(&p9_tgetattr(2, 1, u64::MAX)).unwrap();

        assert_eq!(response.message_type(), P9_RGETATTR);
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.valid, u64::MAX);
        assert_eq!(attr.qid.qid_type, 0x80);
        assert_eq!(attr.mode, P9_MODE_DIR | 0o755);
        assert_eq!(attr.size, 4);
    }

    #[test]
    fn getattr_unknown_fid_returns_bad_fd() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(fs);

        let response = server.handle_frame(&p9_tgetattr(1, 99, 0)).unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn walk_missing_path_returns_lerror() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_twalk(2, 1, 2, &["missing"]).unwrap())
            .unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, 2);
    }

    #[test]
    fn version_resets_fid_table() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tversion(2, 4096, P9_VERSION_9P2000_L).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);

        let response = server
            .handle_frame(&p9_twalk(3, 1, 2, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn zero_name_walk_clones_fid_path() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_twalk(2, 1, 2, &[]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        assert!(p9_decode_rwalk(&response).unwrap().is_empty());

        let response = server
            .handle_frame(&p9_twalk(3, 2, 3, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
    }

    fn server(fs: Arc<MemFs>) -> P9Server {
        let root: Arc<dyn FileSystem> = fs;
        P9Server::new(root)
    }

    fn attach_root(server: &mut P9Server) {
        let response = server
            .handle_frame(&p9_tattach(1, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);
    }

    fn walk(server: &mut P9Server, fid: u32, newfid: u32, names: &[&str]) {
        let response = server
            .handle_frame(&p9_twalk(2, fid, newfid, names).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
    }

    fn entry_names(entries: &[P9DirEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
