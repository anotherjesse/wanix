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
use std::time::{SystemTime, UNIX_EPOCH};

use wanix_fs::{
    File, FileSeekFrom, FileSystem, FileType, FsError, Metadata, MetadataLookup, NormalizedPath,
    OpenOptions,
};
use wanix_protocol::{
    P9_SETATTR_ATIME, P9_SETATTR_ATIME_NOT_SYSTEM_TIME, P9_SETATTR_CTIME, P9_SETATTR_GID,
    P9_SETATTR_MTIME, P9_SETATTR_MTIME_NOT_SYSTEM_TIME, P9_SETATTR_PERMISSIONS, P9_SETATTR_SIZE,
    P9_SETATTR_UID, P9_TATTACH, P9_TAUTH, P9_TCLUNK, P9_TFLUSH, P9_TFLUSHF, P9_TFSYNC, P9_TGETATTR,
    P9_TGETLOCK, P9_TLCREATE, P9_TLINK, P9_TLOCK, P9_TLOPEN, P9_TMKDIR, P9_TMKNOD, P9_TREAD,
    P9_TREADDIR, P9_TREADLINK, P9_TREMOVE, P9_TRENAME, P9_TRENAMEAT, P9_TSETATTR, P9_TSTATFS,
    P9_TSYMLINK, P9_TUNLINKAT, P9_TVERSION, P9_TWALK, P9_TWALKGETATTR, P9_TWRITE, P9_TXATTRCREATE,
    P9_TXATTRWALK, P9_VERSION_9P2000_L, P9_VERSION_9P2000_L_GOOGLE_1, P9_VERSION_9P2000_L_GOOGLE_2,
    P9Attr, P9AttrBody, P9DirEntry, P9Error, P9Frame, P9FsStat, P9Lock, P9Qid, P9SetAttr,
    P9Version, p9_decode_tattach, p9_decode_tauth, p9_decode_tclunk, p9_decode_tflush,
    p9_decode_tflushf, p9_decode_tfsync, p9_decode_tgetattr, p9_decode_tgetlock,
    p9_decode_tlcreate, p9_decode_tlink, p9_decode_tlock, p9_decode_tlopen, p9_decode_tmkdir,
    p9_decode_tmknod, p9_decode_tread, p9_decode_treaddir, p9_decode_treadlink, p9_decode_tremove,
    p9_decode_trename, p9_decode_trenameat, p9_decode_tsetattr, p9_decode_tstatfs,
    p9_decode_tsymlink, p9_decode_tunlinkat, p9_decode_tversion, p9_decode_twalk,
    p9_decode_twalkgetattr, p9_decode_twrite, p9_decode_txattrcreate, p9_decode_txattrwalk,
    p9_dir_entry_encoded_len, p9_rattach, p9_rclunk, p9_rflush, p9_rflushf, p9_rfsync, p9_rgetattr,
    p9_rgetlock, p9_rlcreate, p9_rlerror, p9_rlock, p9_rlopen, p9_rmkdir, p9_rread, p9_rreaddir,
    p9_rreadlink, p9_rremove, p9_rrename, p9_rrenameat, p9_rsetattr, p9_rstatfs, p9_rsymlink,
    p9_runlinkat, p9_rversion, p9_rwalk, p9_rwalkgetattr, p9_rwrite,
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
const ENOSYS: u32 = 38;
const ENOTEMPTY: u32 = 39;
const EOPNOTSUPP: u32 = 95;

const O_ACCMODE: u32 = 0o3;
const O_WRONLY: u32 = 0o1;
const O_RDWR: u32 = 0o2;
const O_CREAT: u32 = 0o100;
const O_TRUNC: u32 = 0o1000;
const O_APPEND: u32 = 0o2000;
const AT_REMOVEDIR: u32 = 0x200;

const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;
const P9_SETATTR_KNOWN_MASK: u32 = P9_SETATTR_PERMISSIONS
    | P9_SETATTR_UID
    | P9_SETATTR_GID
    | P9_SETATTR_SIZE
    | P9_SETATTR_ATIME
    | P9_SETATTR_MTIME
    | P9_SETATTR_CTIME
    | P9_SETATTR_ATIME_NOT_SYSTEM_TIME
    | P9_SETATTR_MTIME_NOT_SYSTEM_TIME;
const P9_SETATTR_OWNER_MASK: u32 = P9_SETATTR_UID | P9_SETATTR_GID;
const P9_SETATTR_UNSUPPORTED_MASK: u32 = P9_SETATTR_CTIME;

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
const P9_GOOGLE_VERSION_PREFIX: &str = "9P2000.L.Google.";
const P9_GOOGLE_TFLUSHF_VERSION: u32 = 1;
const P9_GOOGLE_TWALKGETATTR_VERSION: u32 = 2;
const P9_LOCK_TYPE_UNLOCK: u8 = wanix_protocol::P9_LOCK_TYPE_UNLOCK;
const P9_LOCK_STATUS_OK: u8 = wanix_protocol::P9_LOCK_STATUS_OK;

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
    owners: BTreeMap<NormalizedPath, P9OwnerAttrs>,
    msize: u32,
    max_msize: u32,
    google_version: u32,
}

struct FidEntry {
    path: NormalizedPath,
    file: Option<Box<dyn File>>,
    append: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct P9OwnerAttrs {
    uid: Option<u32>,
    gid: Option<u32>,
}

impl P9Server {
    /// Creates a server around `root` using [`DEFAULT_MAX_MSIZE`].
    #[must_use]
    pub fn new(root: Arc<dyn FileSystem>) -> Self {
        Self {
            root,
            fids: BTreeMap::new(),
            owners: BTreeMap::new(),
            msize: DEFAULT_MAX_MSIZE,
            max_msize: DEFAULT_MAX_MSIZE,
            google_version: 0,
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
            P9_TAUTH => self.handle_auth(frame),
            P9_TATTACH => self.handle_attach(frame),
            P9_TFLUSH => self.handle_flush(frame),
            P9_TFLUSHF => self.handle_flushf(frame),
            P9_TWALK => self.handle_walk(frame),
            P9_TWALKGETATTR => self.handle_walkgetattr(frame),
            P9_TLOPEN => self.handle_open(frame),
            P9_TLCREATE => self.handle_create(frame),
            P9_TSYMLINK => self.handle_symlink(frame),
            P9_TMKNOD => self.handle_mknod(frame),
            P9_TREADLINK => self.handle_readlink(frame),
            P9_TGETATTR => self.handle_getattr(frame),
            P9_TSETATTR => self.handle_setattr(frame),
            P9_TXATTRWALK => self.handle_xattrwalk(frame),
            P9_TXATTRCREATE => self.handle_xattrcreate(frame),
            P9_TREADDIR => self.handle_readdir(frame),
            P9_TFSYNC => self.handle_fsync(frame),
            P9_TLOCK => self.handle_lock(frame),
            P9_TGETLOCK => self.handle_getlock(frame),
            P9_TREAD => self.handle_read(frame),
            P9_TWRITE => self.handle_write(frame),
            P9_TLINK => self.handle_link(frame),
            P9_TMKDIR => self.handle_mkdir(frame),
            P9_TRENAME => self.handle_rename(frame),
            P9_TRENAMEAT => self.handle_renameat(frame),
            P9_TREMOVE => self.handle_remove(frame),
            P9_TUNLINKAT => self.handle_unlinkat(frame),
            P9_TCLUNK => self.handle_clunk(frame),
            _ => Ok(p9_rlerror(frame.tag(), EOPNOTSUPP)),
        }
    }

    fn handle_version(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let P9Version { msize, version } = p9_decode_tversion(frame)?;
        self.fids.clear();
        self.owners.clear();
        self.msize = msize.min(self.max_msize);
        let (version, google_version) = negotiate_p9_version(&version);
        self.google_version = google_version;
        Ok(p9_rversion(frame.tag(), self.msize, version)?)
    }

    fn handle_attach(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let attach = p9_decode_tattach(frame)?;
        let path = NormalizedPath::new(".").expect("root path is valid");
        let qid = match self.qid_for_path(&path) {
            Ok(qid) => qid,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        self.fids.insert(
            attach.fid,
            FidEntry {
                path,
                file: None,
                append: false,
            },
        );
        Ok(p9_rattach(frame.tag(), qid))
    }

    fn handle_flush(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        p9_decode_tflush(frame)?;
        Ok(p9_rflush(frame.tag()))
    }

    fn handle_flushf(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let flushf = p9_decode_tflushf(frame)?;
        if self.google_version < P9_GOOGLE_TFLUSHF_VERSION {
            return Ok(p9_rlerror(frame.tag(), EOPNOTSUPP));
        }
        if self.fids.contains_key(&flushf.fid) {
            Ok(p9_rflushf(frame.tag()))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
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

    fn handle_auth(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        p9_decode_tauth(frame)?;
        Ok(p9_rlerror(frame.tag(), ENOSYS))
    }

    fn handle_walk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let walk = p9_decode_twalk(frame)?;
        let Some(source_path) = self.fids.get(&walk.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let mut path = source_path;
        let mut qids = Vec::with_capacity(walk.names.len());
        for name in &walk.names {
            path = join_walk_component(&path, name)?;
            match self.qid_for_path(&path) {
                Ok(qid) => qids.push(qid),
                Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
            }
        }
        self.fids.insert(
            walk.newfid,
            FidEntry {
                path,
                file: None,
                append: false,
            },
        );
        Ok(p9_rwalk(frame.tag(), &qids)?)
    }

    fn handle_walkgetattr(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let walk = p9_decode_twalkgetattr(frame)?;
        if self.google_version < P9_GOOGLE_TWALKGETATTR_VERSION {
            return Ok(p9_rlerror(frame.tag(), EOPNOTSUPP));
        }
        let Some(source_path) = self.fids.get(&walk.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let mut path = source_path;
        let mut qids = Vec::with_capacity(walk.names.len());
        for name in &walk.names {
            path = join_walk_component(&path, name)?;
            match self.qid_for_path(&path) {
                Ok(qid) => qids.push(qid),
                Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
            }
        }
        let metadata = match self.metadata_no_follow(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let attr = attr_for_metadata(&path, metadata, u64::MAX, self.owner_attrs(&path));
        self.fids.insert(
            walk.newfid,
            FidEntry {
                path,
                file: None,
                append: false,
            },
        );
        Ok(p9_rwalkgetattr(
            frame.tag(),
            attr.valid,
            &P9AttrBody::from(&attr),
            &qids,
        )?)
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
        let is_directory = metadata.file_type() == FileType::Directory;
        let append = !is_directory && open.flags & O_APPEND != 0;
        let file = if is_directory {
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
        entry.append = append;
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
        let existing_path = self.metadata_no_follow(&path).is_ok();
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
        if !existing_path {
            self.set_created_gid(&path, create.gid);
        }
        let Some(entry) = self.fids.get_mut(&create.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        entry.path = path;
        entry.file = Some(file);
        entry.append = create.flags & O_APPEND != 0;
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
        self.set_created_gid(&path, symlink.gid);
        Ok(p9_rsymlink(frame.tag(), qid_for_metadata(&path, metadata)))
    }

    fn handle_mknod(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let mknod = p9_decode_tmknod(frame)?;
        let Some(dir_path) = self
            .fids
            .get(&mknod.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let _path = join_walk_component(&dir_path, &mknod.name)?;
        Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
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
        let attr = attr_for_metadata(
            &path,
            metadata,
            getattr.request_mask,
            self.owner_attrs(&path),
        );
        Ok(p9_rgetattr(frame.tag(), &attr))
    }

    fn handle_setattr(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let setattr = p9_decode_tsetattr(frame)?;
        let Some(path) = self.fids.get(&setattr.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if setattr.valid & !P9_SETATTR_KNOWN_MASK != 0 {
            return Ok(p9_rlerror(frame.tag(), EINVAL));
        }
        if setattr.valid & P9_SETATTR_UNSUPPORTED_MASK != 0 {
            return Ok(p9_rlerror(frame.tag(), EOPNOTSUPP));
        }
        if let Err(error) = validate_setattr_times(setattr.valid, &setattr.attr) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        if setattr.valid & P9_SETATTR_OWNER_MASK != 0
            && let Err(error) = self.metadata_no_follow(&path)
        {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }

        if setattr.valid & P9_SETATTR_SIZE != 0
            && let Err(error) = self.set_file_size(&path, setattr.attr.size)
        {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        if setattr.valid & (P9_SETATTR_ATIME | P9_SETATTR_MTIME) != 0
            && let Err(error) = self.set_file_times(&path, setattr.valid, &setattr.attr)
        {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        if setattr.valid & P9_SETATTR_PERMISSIONS != 0
            && let Err(error) = self.root.set_permissions(&path, setattr.attr.permissions)
        {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        if setattr.valid & P9_SETATTR_OWNER_MASK != 0 {
            self.set_owner_attrs(&path, setattr.valid, &setattr.attr);
        }
        Ok(p9_rsetattr(frame.tag()))
    }

    fn handle_xattrwalk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let xattr = p9_decode_txattrwalk(frame)?;
        if self.fids.contains_key(&xattr.fid) {
            Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    fn handle_xattrcreate(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let xattr = p9_decode_txattrcreate(frame)?;
        if self.fids.contains_key(&xattr.fid) {
            Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
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

    fn handle_fsync(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let fsync = p9_decode_tfsync(frame)?;
        if self.fids.contains_key(&fsync.fid) {
            Ok(p9_rfsync(frame.tag()))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    fn handle_lock(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let lock = p9_decode_tlock(frame)?;
        if self.fids.contains_key(&lock.fid) {
            Ok(p9_rlock(frame.tag(), P9_LOCK_STATUS_OK))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    fn handle_getlock(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let getlock = p9_decode_tgetlock(frame)?;
        if !self.fids.contains_key(&getlock.fid) {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        }
        Ok(p9_rgetlock(
            frame.tag(),
            &P9Lock {
                lock_type: P9_LOCK_TYPE_UNLOCK,
                start: getlock.lock.start,
                length: getlock.lock.length,
                proc_id: 0,
                client_id: String::new(),
            },
        )?)
    }

    fn handle_read(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let read = p9_decode_tread(frame)?;
        let Some(entry) = self.fids.get_mut(&read.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if file.is_seekable()
            && let Err(error) = file.seek(FileSeekFrom::Start(read.offset))
        {
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
        let append = entry.append;
        let Some(file) = entry.file.as_mut() else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let seek_from = if append {
            FileSeekFrom::End(0)
        } else {
            FileSeekFrom::Start(write.offset)
        };
        if file.is_seekable()
            && let Err(error) = file.seek(seek_from)
        {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        let count = match file.write(&write.data) {
            Ok(count) => count,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        Ok(p9_rwrite(frame.tag(), count as u32))
    }

    fn handle_link(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let link = p9_decode_tlink(frame)?;
        let Some(dir_path) = self.fids.get(&link.dir_fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if !self.fids.contains_key(&link.fid) {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        }
        let _path = join_walk_component(&dir_path, &link.name)?;
        Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
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
        self.set_created_gid(&path, mkdir.gid);
        Ok(p9_rmkdir(frame.tag(), qid_for_metadata(&path, metadata)))
    }

    fn handle_rename(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let rename = p9_decode_trename(frame)?;
        let Some(old_path) = self.fids.get(&rename.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let Some(new_dir_path) = self
            .fids
            .get(&rename.dir_fid)
            .map(|entry| entry.path.clone())
        else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let new_path = join_walk_component(&new_dir_path, &rename.name)?;
        match self.root.rename(&old_path, &new_path) {
            Ok(()) => {
                self.move_owner_attrs(&old_path, &new_path);
                self.remove_replaced_fid_paths(&old_path, &new_path);
                self.move_fid_paths(&old_path, &new_path);
                Ok(p9_rrename(frame.tag()))
            }
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
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
            Ok(()) => {
                self.move_owner_attrs(&old_path, &new_path);
                self.remove_replaced_fid_paths(&old_path, &new_path);
                self.move_fid_paths(&old_path, &new_path);
                Ok(p9_rrenameat(frame.tag()))
            }
            Err(error) => Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        }
    }

    fn handle_remove(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let remove = p9_decode_tremove(frame)?;
        let Some(entry) = self.fids.remove(&remove.fid) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        let path = entry.path;
        let metadata = match self.metadata_no_follow(&path) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        let result = if metadata.file_type() == FileType::Directory {
            self.root.remove_dir(&path)
        } else {
            self.root.remove_file(&path)
        };
        match result {
            Ok(()) => {
                self.remove_owner_attrs(&path);
                Ok(p9_rremove(frame.tag()))
            }
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
            Ok(()) => {
                self.remove_owner_attrs(&path);
                Ok(p9_runlinkat(frame.tag()))
            }
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

    fn owner_attrs(&self, path: &NormalizedPath) -> P9OwnerAttrs {
        self.owners.get(path).copied().unwrap_or_default()
    }

    fn set_owner_attrs(&mut self, path: &NormalizedPath, valid: u32, attr: &P9SetAttr) {
        let mut owner = self.owner_attrs(path);
        if valid & P9_SETATTR_UID != 0 {
            owner.uid = Some(attr.uid);
        }
        if valid & P9_SETATTR_GID != 0 {
            owner.gid = Some(attr.gid);
        }
        self.owners.insert(path.clone(), owner);
    }

    fn set_created_gid(&mut self, path: &NormalizedPath, gid: u32) {
        self.owners.insert(
            path.clone(),
            P9OwnerAttrs {
                uid: None,
                gid: Some(gid),
            },
        );
    }

    fn move_owner_attrs(&mut self, old_path: &NormalizedPath, new_path: &NormalizedPath) {
        if old_path == new_path {
            return;
        }
        let moves = self
            .owners
            .iter()
            .filter_map(|(path, owner)| {
                rebase_path(path, old_path, new_path).map(|to| (path.clone(), to, *owner))
            })
            .collect::<Vec<_>>();
        self.remove_owner_attrs(new_path);
        for (from, to, owner) in moves {
            self.owners.remove(&from);
            self.owners.insert(to, owner);
        }
    }

    fn move_fid_paths(&mut self, old_path: &NormalizedPath, new_path: &NormalizedPath) {
        if old_path == new_path {
            return;
        }
        for entry in self.fids.values_mut() {
            if let Some(to) = rebase_path(&entry.path, old_path, new_path) {
                entry.path = to;
            }
        }
    }

    fn remove_replaced_fid_paths(&mut self, old_path: &NormalizedPath, new_path: &NormalizedPath) {
        if old_path == new_path {
            return;
        }
        self.fids
            .retain(|_, entry| !is_same_or_descendant_path(&entry.path, new_path));
    }

    fn remove_owner_attrs(&mut self, removed_path: &NormalizedPath) {
        self.owners
            .retain(|path, _| !is_same_or_descendant_path(path, removed_path));
    }

    fn set_file_size(&self, path: &NormalizedPath, size: u64) -> Result<(), FsError> {
        let mut file = self.root.open(
            path,
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )?;
        file.set_len(size)
    }

    fn set_file_times(
        &self,
        path: &NormalizedPath,
        valid: u32,
        attr: &P9SetAttr,
    ) -> Result<(), FsError> {
        let metadata = self.root.metadata(path)?;
        let now = if requests_system_time(valid) {
            Some(current_unix_time_ns()?)
        } else {
            None
        };
        let accessed_time_ns = if valid & P9_SETATTR_ATIME == 0 {
            metadata.accessed_time_ns()
        } else if valid & P9_SETATTR_ATIME_NOT_SYSTEM_TIME != 0 {
            unix_time_ns(attr.atime_seconds, attr.atime_nanoseconds)?
        } else {
            now.expect("system access time was requested")
        };
        let modified_time_ns = if valid & P9_SETATTR_MTIME == 0 {
            metadata.modified_time_ns()
        } else if valid & P9_SETATTR_MTIME_NOT_SYSTEM_TIME != 0 {
            unix_time_ns(attr.mtime_seconds, attr.mtime_nanoseconds)?
        } else {
            now.expect("system modification time was requested")
        };
        self.root
            .set_times(path, accessed_time_ns, modified_time_ns)
    }
}

fn validate_setattr_times(valid: u32, attr: &P9SetAttr) -> Result<(), FsError> {
    if valid & P9_SETATTR_ATIME != 0 && valid & P9_SETATTR_ATIME_NOT_SYSTEM_TIME != 0 {
        unix_time_ns(attr.atime_seconds, attr.atime_nanoseconds)?;
    }
    if valid & P9_SETATTR_MTIME != 0 && valid & P9_SETATTR_MTIME_NOT_SYSTEM_TIME != 0 {
        unix_time_ns(attr.mtime_seconds, attr.mtime_nanoseconds)?;
    }
    Ok(())
}

fn negotiate_p9_version(requested: &str) -> (&'static str, u32) {
    if requested == P9_VERSION_9P2000_L {
        return (P9_VERSION_9P2000_L, 0);
    }
    let Some(version) = requested
        .strip_prefix(P9_GOOGLE_VERSION_PREFIX)
        .and_then(|suffix| suffix.parse::<u32>().ok())
    else {
        return ("unknown", 0);
    };
    if version >= P9_GOOGLE_TWALKGETATTR_VERSION {
        (P9_VERSION_9P2000_L_GOOGLE_2, P9_GOOGLE_TWALKGETATTR_VERSION)
    } else if version >= P9_GOOGLE_TFLUSHF_VERSION {
        (P9_VERSION_9P2000_L_GOOGLE_1, P9_GOOGLE_TFLUSHF_VERSION)
    } else {
        (P9_VERSION_9P2000_L, 0)
    }
}

fn requests_system_time(valid: u32) -> bool {
    valid & P9_SETATTR_ATIME != 0 && valid & P9_SETATTR_ATIME_NOT_SYSTEM_TIME == 0
        || valid & P9_SETATTR_MTIME != 0 && valid & P9_SETATTR_MTIME_NOT_SYSTEM_TIME == 0
}

fn unix_time_ns(seconds: u64, nanoseconds: u64) -> Result<u64, FsError> {
    if nanoseconds >= NANOSECONDS_PER_SECOND {
        return Err(FsError::InvalidTime);
    }
    seconds
        .checked_mul(NANOSECONDS_PER_SECOND)
        .and_then(|base| base.checked_add(nanoseconds))
        .ok_or(FsError::InvalidTime)
}

fn current_unix_time_ns() -> Result<u64, FsError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| FsError::InvalidTime)?;
    u64::try_from(duration.as_nanos()).map_err(|_| FsError::InvalidTime)
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

fn attr_for_metadata(
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

fn rebase_path(
    path: &NormalizedPath,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
) -> Option<NormalizedPath> {
    if path == old_path {
        return Some(new_path.clone());
    }
    let suffix = descendant_suffix(path, old_path)?;
    let rebased = if new_path.as_str() == "." {
        suffix.to_owned()
    } else {
        format!("{}/{}", new_path.as_str(), suffix)
    };
    NormalizedPath::new(&rebased).ok()
}

fn is_same_or_descendant_path(path: &NormalizedPath, base: &NormalizedPath) -> bool {
    path == base || descendant_suffix(path, base).is_some()
}

fn descendant_suffix<'a>(path: &'a NormalizedPath, base: &NormalizedPath) -> Option<&'a str> {
    let base = base.as_str();
    if base == "." {
        return Some(path.as_str());
    }
    path.as_str()
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|suffix| !suffix.is_empty())
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
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use wanix_fs::{DirEntry, FsResult, LocalFs, MemFs};
    use wanix_protocol::{
        P9_LOCK_TYPE_READ, P9_LOCK_TYPE_WRITE, P9_RATTACH, P9_RFLUSH, P9_RFLUSHF, P9_RFSYNC,
        P9_RGETATTR, P9_RGETLOCK, P9_RLCREATE, P9_RLERROR, P9_RLOCK, P9_RLOPEN, P9_RMKDIR,
        P9_RREAD, P9_RREADDIR, P9_RREADLINK, P9_RREMOVE, P9_RRENAME, P9_RRENAMEAT, P9_RSETATTR,
        P9_RSTATFS, P9_RSYMLINK, P9_RUNLINKAT, P9_RVERSION, P9_RWALK, P9_RWALKGETATTR, P9_RWRITE,
        P9_SETATTR_ATIME, P9_SETATTR_ATIME_NOT_SYSTEM_TIME, P9_SETATTR_MTIME,
        P9_SETATTR_MTIME_NOT_SYSTEM_TIME, P9_SETATTR_PERMISSIONS, P9_SETATTR_SIZE,
        P9_VERSION_9P2000_L_GOOGLE_1, P9_VERSION_9P2000_L_GOOGLE_2, P9DirEntry, P9Lock, P9SetAttr,
        p9_decode_rflush, p9_decode_rflushf, p9_decode_rfsync, p9_decode_rgetattr,
        p9_decode_rgetlock, p9_decode_rlcreate, p9_decode_rlerror, p9_decode_rlock,
        p9_decode_rlopen, p9_decode_rmkdir, p9_decode_rread, p9_decode_rreaddir,
        p9_decode_rreadlink, p9_decode_rremove, p9_decode_rrename, p9_decode_rsetattr,
        p9_decode_rstatfs, p9_decode_rsymlink, p9_decode_rversion, p9_decode_rwalk,
        p9_decode_rwalkgetattr, p9_decode_rwrite, p9_dir_entry_encoded_len, p9_tattach, p9_tauth,
        p9_tclunk, p9_tflush, p9_tflushf, p9_tfsync, p9_tgetattr, p9_tgetlock, p9_tlcreate,
        p9_tlink, p9_tlock, p9_tlopen, p9_tmkdir, p9_tmknod, p9_tread, p9_treaddir, p9_treadlink,
        p9_tremove, p9_trename, p9_trenameat, p9_tsetattr, p9_tstatfs, p9_tsymlink, p9_tunlinkat,
        p9_tversion, p9_twalk, p9_twalkgetattr, p9_twrite, p9_txattrcreate, p9_txattrwalk,
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
    fn flush_acknowledges_without_fid_lookup() {
        let mut server = server(Arc::new(MemFs::new()));

        let response = server.handle_frame(&p9_tflush(8, 7)).unwrap();

        assert_eq!(response.message_type(), P9_RFLUSH);
        p9_decode_rflush(&response).unwrap();
    }

    #[test]
    fn auth_probe_returns_enosys_without_binding_fid() {
        let mut server = server(Arc::new(MemFs::new()));

        let response = server
            .handle_frame(&p9_tauth(2, 9, "root", "", 0).unwrap())
            .unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, ENOSYS);
        assert!(!server.fids.contains_key(&9));
    }

    #[test]
    fn fsync_existing_fid_returns_success() {
        let mut server = server(Arc::new(MemFs::new()));

        attach_root(&mut server);
        let response = server.handle_frame(&p9_tfsync(2, 1)).unwrap();

        assert_eq!(response.message_type(), P9_RFSYNC);
        p9_decode_rfsync(&response).unwrap();
    }

    #[test]
    fn fsync_unknown_fid_returns_bad_fd() {
        let mut server = server(Arc::new(MemFs::new()));

        let response = server.handle_frame(&p9_tfsync(3, 99)).unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn lock_existing_fid_returns_ok_status() {
        let mut server = server(Arc::new(MemFs::new()));

        attach_root(&mut server);
        let lock = P9Lock {
            lock_type: P9_LOCK_TYPE_WRITE,
            start: 0,
            length: 100,
            proc_id: 123,
            client_id: "linux-client".to_owned(),
        };
        let response = server
            .handle_frame(&p9_tlock(3, 1, 1, &lock).unwrap())
            .unwrap();

        assert_eq!(response.message_type(), P9_RLOCK);
        assert_eq!(p9_decode_rlock(&response).unwrap(), P9_LOCK_STATUS_OK);
    }

    #[test]
    fn getlock_existing_fid_reports_no_conflict() {
        let mut server = server(Arc::new(MemFs::new()));

        attach_root(&mut server);
        let lock = P9Lock {
            lock_type: P9_LOCK_TYPE_READ,
            start: 4,
            length: 8,
            proc_id: 456,
            client_id: "linux-client".to_owned(),
        };
        let response = server
            .handle_frame(&p9_tgetlock(3, 1, &lock).unwrap())
            .unwrap();

        assert_eq!(response.message_type(), P9_RGETLOCK);
        assert_eq!(
            p9_decode_rgetlock(&response).unwrap(),
            P9Lock {
                lock_type: P9_LOCK_TYPE_UNLOCK,
                start: 4,
                length: 8,
                proc_id: 0,
                client_id: String::new()
            }
        );
    }

    #[test]
    fn lock_unknown_fid_returns_bad_fd() {
        let mut server = server(Arc::new(MemFs::new()));
        let lock = P9Lock {
            lock_type: P9_LOCK_TYPE_WRITE,
            start: 0,
            length: 1,
            proc_id: 1,
            client_id: "linux-client".to_owned(),
        };

        let response = server
            .handle_frame(&p9_tlock(3, 99, 0, &lock).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);

        let response = server
            .handle_frame(&p9_tgetlock(4, 99, &lock).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn compatibility_probes_return_typed_lerrors() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("target.txt", b"target").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["target.txt"]);

        let response = server
            .handle_frame(&p9_tmknod(3, 1, "tty0", 0o020620, 4, 0, 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);

        let response = server
            .handle_frame(&p9_tmknod(4, 99, "tty0", 0o020620, 4, 0, 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);

        let response = server
            .handle_frame(&p9_tlink(5, 1, 2, "hard.txt").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);

        let response = server
            .handle_frame(&p9_tlink(6, 1, 99, "hard.txt").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);

        let response = server
            .handle_frame(&p9_txattrwalk(7, 2, 3, "user.foo").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);
        assert!(!server.fids.contains_key(&3));

        walk(&mut server, 1, 4, &[]);
        let response = server
            .handle_frame(&p9_txattrwalk(8, 2, 4, "user.foo").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);
        assert_eq!(server.fids.get(&4).unwrap().path.as_str(), ".");

        let response = server
            .handle_frame(&p9_txattrwalk(9, 99, 3, "user.foo").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);

        let response = server
            .handle_frame(&p9_txattrcreate(10, 2, "user.foo", 12, 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);

        let response = server
            .handle_frame(&p9_txattrcreate(11, 99, "user.foo", 12, 0).unwrap())
            .unwrap();
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
    fn open_append_ignores_write_offsets_and_writes_at_end() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("log.txt", b"base").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["log.txt"]);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(3, 2, O_WRONLY | O_APPEND))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );

        let response = server
            .handle_frame(&p9_twrite(4, 2, 0, b"-one").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 4);

        let response = server
            .handle_frame(&p9_twrite(5, 2, 1, b"-two").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 4);
        assert_eq!(fs.read_file("log.txt").unwrap(), b"base-one-two");
    }

    #[test]
    fn non_seekable_file_reads_and_writes_without_offset_seek() {
        let root: Arc<dyn FileSystem> = Arc::new(StreamFs::new(b"abc"));
        let mut server = P9Server::new(root);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["stream"]);
        assert_eq!(
            server
                .handle_frame(&p9_tlopen(3, 2, O_RDWR))
                .unwrap()
                .message_type(),
            P9_RLOPEN
        );

        let response = server.handle_frame(&p9_tread(4, 2, 99, 2)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"ab");

        let response = server
            .handle_frame(&p9_twrite(5, 2, 42, b"de").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 2);

        let response = server.handle_frame(&p9_tread(6, 2, 0, 8)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"cde");
    }

    #[test]
    fn lcreate_append_sets_created_fid_append_mode() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tlcreate(2, 1, "log.txt", O_WRONLY | O_APPEND, 0o100664, 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLCREATE);

        let response = server
            .handle_frame(&p9_twrite(3, 1, 0, b"first").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 5);

        let response = server
            .handle_frame(&p9_twrite(4, 1, 0, b"second").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 6);
        assert_eq!(fs.read_file("log.txt").unwrap(), b"firstsecond");
    }

    #[test]
    fn read_only_directory_open_tolerates_append_status_flag() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server.handle_frame(&p9_tlopen(2, 1, O_APPEND)).unwrap();

        assert_eq!(response.message_type(), P9_RLOPEN);
        let response = server.handle_frame(&p9_treaddir(3, 1, 0, 4096)).unwrap();
        assert_eq!(response.message_type(), P9_RREADDIR);
        let entries = p9_decode_rreaddir(&response).unwrap();
        assert_eq!(entries[0].name, "hello.txt");
    }

    #[test]
    fn lcreate_creates_file_and_opens_created_fid() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        let response = server
            .handle_frame(&p9_tlcreate(2, 1, "new.txt", O_RDWR, 0o100664, 1234).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLCREATE);
        let (qid, iounit) = p9_decode_rlcreate(&response).unwrap();
        assert_eq!(qid.qid_type, 0);
        assert_eq!(iounit, DEFAULT_MAX_MSIZE - RLOPEN_OVERHEAD);

        let response = server.handle_frame(&p9_tgetattr(3, 1, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        assert_eq!(p9_decode_rgetattr(&response).unwrap().gid, 1234);

        let response = server
            .handle_frame(&p9_twrite(4, 1, 0, b"created over 9p").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWRITE);
        assert_eq!(p9_decode_rwrite(&response).unwrap(), 15);
        assert_eq!(fs.read_file("new.txt").unwrap(), b"created over 9p");
    }

    #[test]
    fn lcreate_existing_file_preserves_virtual_owner() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("existing.txt", b"old").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["existing.txt"]);
        let owner = P9SetAttr {
            uid: 1000,
            gid: 1001,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(3, 2, P9_SETATTR_UID | P9_SETATTR_GID, &owner))
            .unwrap();
        assert_eq!(response.message_type(), P9_RSETATTR);

        let response = server
            .handle_frame(&p9_tlcreate(4, 1, "existing.txt", O_RDWR, 0o100664, 2000).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLCREATE);

        let response = server.handle_frame(&p9_tgetattr(5, 1, u64::MAX)).unwrap();
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.uid, 1000);
        assert_eq!(attr.gid, 1001);
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
    fn renameat_rebases_source_fids_and_invalidates_replaced_destination_fids() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("old.txt", b"old").unwrap();
        fs.write_file("target.txt", b"target").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["old.txt"]);
        walk(&mut server, 1, 3, &["target.txt"]);

        let response = server
            .handle_frame(&p9_trenameat(4, 1, "old.txt", 1, "target.txt").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RRENAMEAT);
        assert_eq!(fs.read_file("target.txt").unwrap(), b"old");

        let response = server.handle_frame(&p9_tgetattr(5, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        assert_eq!(p9_decode_rgetattr(&response).unwrap().size, 3);

        let response = server.handle_frame(&p9_tgetattr(6, 3, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn renameat_directory_rebases_descendant_fids() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("dir/sub/file.txt", b"nested").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["dir", "sub", "file.txt"]);

        let response = server
            .handle_frame(&p9_trenameat(3, 1, "dir", 1, "moved").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RRENAMEAT);
        assert_eq!(fs.read_file("moved/sub/file.txt").unwrap(), b"nested");

        let response = server.handle_frame(&p9_tgetattr(4, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        assert_eq!(p9_decode_rgetattr(&response).unwrap().size, 6);
    }

    #[test]
    fn legacy_rename_moves_file_and_rebases_source_fid() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("old.txt", b"moved").unwrap();
        fs.create_dir_all("dest").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["old.txt"]);
        walk(&mut server, 1, 3, &["dest"]);
        let owner = P9SetAttr {
            uid: 1000,
            gid: 1001,
            ..P9SetAttr::default()
        };
        assert_eq!(
            server
                .handle_frame(&p9_tsetattr(4, 2, P9_SETATTR_UID | P9_SETATTR_GID, &owner))
                .unwrap()
                .message_type(),
            P9_RSETATTR
        );

        let response = server
            .handle_frame(&p9_trename(5, 2, 3, "new.txt").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RRENAME);
        p9_decode_rrename(&response).unwrap();
        assert_eq!(fs.read_file("dest/new.txt").unwrap(), b"moved");
        assert_eq!(
            fs.metadata(&NormalizedPath::new("old.txt").unwrap()),
            Err(FsError::NotFound)
        );

        let response = server.handle_frame(&p9_tgetattr(6, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.uid, 1000);
        assert_eq!(attr.gid, 1001);
    }

    #[test]
    fn legacy_remove_deletes_file_clears_owner_and_clunks_fid() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("gone.txt", b"gone").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["gone.txt"]);
        let owner = P9SetAttr {
            uid: 42,
            gid: 43,
            ..P9SetAttr::default()
        };
        assert_eq!(
            server
                .handle_frame(&p9_tsetattr(3, 2, P9_SETATTR_UID | P9_SETATTR_GID, &owner))
                .unwrap()
                .message_type(),
            P9_RSETATTR
        );

        let response = server.handle_frame(&p9_tremove(4, 2)).unwrap();
        assert_eq!(response.message_type(), P9_RREMOVE);
        p9_decode_rremove(&response).unwrap();
        assert_eq!(
            fs.metadata(&NormalizedPath::new("gone.txt").unwrap()),
            Err(FsError::NotFound)
        );

        let response = server.handle_frame(&p9_tgetattr(5, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);

        fs.write_file("gone.txt", b"fresh").unwrap();
        walk(&mut server, 1, 3, &["gone.txt"]);
        let response = server.handle_frame(&p9_tgetattr(6, 3, u64::MAX)).unwrap();
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.uid, 0);
        assert_eq!(attr.gid, 0);
    }

    #[test]
    fn legacy_remove_deletes_empty_directory_and_clunks_fid() {
        let fs = Arc::new(MemFs::new());
        fs.create_dir_all("empty").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["empty"]);

        let response = server.handle_frame(&p9_tremove(3, 2)).unwrap();
        assert_eq!(response.message_type(), P9_RREMOVE);
        p9_decode_rremove(&response).unwrap();
        assert_eq!(
            fs.metadata(&NormalizedPath::new("empty").unwrap()),
            Err(FsError::NotFound)
        );

        let response = server.handle_frame(&p9_tgetattr(4, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn legacy_remove_clunks_fid_even_when_remove_fails() {
        let fs = Arc::new(MemFs::new());
        let mut server = server(fs);

        attach_root(&mut server);
        let response = server.handle_frame(&p9_tremove(2, 1)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EACCES);

        let response = server.handle_frame(&p9_tgetattr(3, 1, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
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
    fn setattr_size_and_explicit_times_mutate_file() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("resize.txt", b"abcdef").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["resize.txt"]);
        let attr = P9SetAttr {
            size: 3,
            atime_seconds: 4,
            atime_nanoseconds: 5,
            mtime_seconds: 6,
            mtime_nanoseconds: 7,
            ..P9SetAttr::default()
        };
        let valid = P9_SETATTR_SIZE
            | P9_SETATTR_ATIME
            | P9_SETATTR_ATIME_NOT_SYSTEM_TIME
            | P9_SETATTR_MTIME
            | P9_SETATTR_MTIME_NOT_SYSTEM_TIME;
        let response = server
            .handle_frame(&p9_tsetattr(3, 2, valid, &attr))
            .unwrap();

        assert_eq!(response.message_type(), P9_RSETATTR);
        p9_decode_rsetattr(&response).unwrap();
        assert_eq!(fs.read_file("resize.txt").unwrap(), b"abc");
        let metadata = fs
            .metadata(&NormalizedPath::new("resize.txt").unwrap())
            .unwrap();
        assert_eq!(metadata.accessed_time_ns(), 4_000_000_005);
        assert_eq!(metadata.modified_time_ns(), 6_000_000_007);
    }

    #[test]
    fn setattr_size_mutates_host_backed_file() {
        let root = temp_dir("wanix-9p-setattr-size");
        fs::write(root.join("host.txt"), b"abcdef").unwrap();
        let local = Arc::new(LocalFs::new(&root).unwrap());
        let root_fs: Arc<dyn FileSystem> = local;
        let mut server = P9Server::new(root_fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["host.txt"]);
        let attr = P9SetAttr {
            size: 2,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(3, 2, P9_SETATTR_SIZE, &attr))
            .unwrap();

        assert_eq!(response.message_type(), P9_RSETATTR);
        assert_eq!(fs::read(root.join("host.txt")).unwrap(), b"ab");
    }

    #[test]
    fn setattr_unknown_fid_returns_bad_fd() {
        let mut server = server(Arc::new(MemFs::new()));

        let response = server
            .handle_frame(&p9_tsetattr(1, 99, 0, &P9SetAttr::default()))
            .unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn setattr_invalid_time_does_not_partially_resize() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("time.txt", b"abcdef").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["time.txt"]);
        let attr = P9SetAttr {
            size: 3,
            atime_nanoseconds: NANOSECONDS_PER_SECOND,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(
                3,
                2,
                P9_SETATTR_SIZE | P9_SETATTR_ATIME | P9_SETATTR_ATIME_NOT_SYSTEM_TIME,
                &attr,
            ))
            .unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EINVAL);
        assert_eq!(fs.read_file("time.txt").unwrap(), b"abcdef");
    }

    #[test]
    fn setattr_permissions_update_reported_mode() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("mode.txt", b"abcdef").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["mode.txt"]);
        let attr = P9SetAttr {
            permissions: 0o600,
            size: 3,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(
                3,
                2,
                P9_SETATTR_PERMISSIONS | P9_SETATTR_SIZE,
                &attr,
            ))
            .unwrap();

        assert_eq!(response.message_type(), P9_RSETATTR);
        assert_eq!(fs.read_file("mode.txt").unwrap(), b"abc");
        assert_eq!(
            fs.metadata(&NormalizedPath::new("mode.txt").unwrap())
                .unwrap()
                .mode(),
            0o600
        );

        let response = server.handle_frame(&p9_tgetattr(4, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        assert_eq!(
            p9_decode_rgetattr(&response).unwrap().mode,
            P9_MODE_REG | 0o600
        );
    }

    #[test]
    #[cfg(unix)]
    fn setattr_permissions_mutate_host_backed_file_mode() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("wanix-9p-setattr-permissions");
        fs::write(root.join("host.txt"), b"abcdef").unwrap();
        let local = Arc::new(LocalFs::new(&root).unwrap());
        let root_fs: Arc<dyn FileSystem> = local;
        let mut server = P9Server::new(root_fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["host.txt"]);
        let attr = P9SetAttr {
            permissions: 0o600,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(3, 2, P9_SETATTR_PERMISSIONS, &attr))
            .unwrap();

        assert_eq!(response.message_type(), P9_RSETATTR);
        assert_eq!(
            fs::metadata(root.join("host.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn setattr_uid_gid_update_reported_owner() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("owner.txt", b"abcdef").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["owner.txt"]);
        let attr = P9SetAttr {
            uid: 1000,
            gid: 1001,
            size: 3,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(
                3,
                2,
                P9_SETATTR_UID | P9_SETATTR_GID | P9_SETATTR_SIZE,
                &attr,
            ))
            .unwrap();

        assert_eq!(response.message_type(), P9_RSETATTR);
        p9_decode_rsetattr(&response).unwrap();
        assert_eq!(fs.read_file("owner.txt").unwrap(), b"abc");

        let response = server.handle_frame(&p9_tgetattr(4, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.uid, 1000);
        assert_eq!(attr.gid, 1001);
    }

    #[test]
    fn virtual_owner_attrs_move_on_rename_and_clear_on_unlink() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("old.txt", b"owned").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["old.txt"]);
        let attr = P9SetAttr {
            uid: 42,
            gid: 43,
            ..P9SetAttr::default()
        };
        assert_eq!(
            server
                .handle_frame(&p9_tsetattr(3, 2, P9_SETATTR_UID | P9_SETATTR_GID, &attr))
                .unwrap()
                .message_type(),
            P9_RSETATTR
        );

        let response = server
            .handle_frame(&p9_trenameat(4, 1, "old.txt", 1, "new.txt").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RRENAMEAT);
        walk(&mut server, 1, 3, &["new.txt"]);
        let response = server.handle_frame(&p9_tgetattr(5, 3, u64::MAX)).unwrap();
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.uid, 42);
        assert_eq!(attr.gid, 43);

        let response = server
            .handle_frame(&p9_tunlinkat(6, 1, "new.txt", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RUNLINKAT);
        fs.write_file("new.txt", b"fresh").unwrap();
        walk(&mut server, 1, 4, &["new.txt"]);
        let response = server.handle_frame(&p9_tgetattr(7, 4, u64::MAX)).unwrap();
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.uid, 0);
        assert_eq!(attr.gid, 0);
    }

    #[test]
    fn setattr_ctime_remains_unsupported_without_partial_resize() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("ctime.txt", b"abcdef").unwrap();
        let mut server = server(Arc::clone(&fs));

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["ctime.txt"]);
        let attr = P9SetAttr {
            size: 3,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(
                3,
                2,
                P9_SETATTR_CTIME | P9_SETATTR_SIZE,
                &attr,
            ))
            .unwrap();

        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);
        assert_eq!(fs.read_file("ctime.txt").unwrap(), b"abcdef");
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
    fn version_negotiates_google_extensions_and_caps_future_versions() {
        let mut server = server(Arc::new(MemFs::new()));

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L_GOOGLE_1).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        assert_eq!(
            p9_decode_rversion(&response).unwrap().version,
            P9_VERSION_9P2000_L_GOOGLE_1
        );

        let response = server
            .handle_frame(&p9_tversion(2, 8192, "9P2000.L.Google.7").unwrap())
            .unwrap();
        assert_eq!(
            p9_decode_rversion(&response).unwrap().version,
            P9_VERSION_9P2000_L_GOOGLE_2
        );

        let response = server
            .handle_frame(&p9_tversion(3, 8192, "9P2000.u").unwrap())
            .unwrap();
        assert_eq!(p9_decode_rversion(&response).unwrap().version, "unknown");
    }

    #[test]
    fn version_resets_virtual_owner_attrs() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("owned.txt", b"owned").unwrap();
        let mut server = server(fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["owned.txt"]);
        let owner = P9SetAttr {
            uid: 7,
            gid: 8,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(3, 2, P9_SETATTR_UID | P9_SETATTR_GID, &owner))
            .unwrap();
        assert_eq!(response.message_type(), P9_RSETATTR);

        let response = server
            .handle_frame(&p9_tversion(4, 8192, P9_VERSION_9P2000_L).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        attach_root(&mut server);
        walk(&mut server, 1, 3, &["owned.txt"]);
        let response = server.handle_frame(&p9_tgetattr(5, 3, u64::MAX)).unwrap();
        let attr = p9_decode_rgetattr(&response).unwrap();
        assert_eq!(attr.uid, 0);
        assert_eq!(attr.gid, 0);
    }

    #[test]
    fn flushf_is_gated_by_google_1_and_validates_fids() {
        let mut server = server(Arc::new(MemFs::new()));

        attach_root(&mut server);
        let response = server.handle_frame(&p9_tflushf(2, 1)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);

        let response = server
            .handle_frame(&p9_tversion(3, 8192, P9_VERSION_9P2000_L_GOOGLE_1).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        let response = server
            .handle_frame(&p9_tattach(4, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);

        let response = server.handle_frame(&p9_tflushf(5, 1)).unwrap();
        assert_eq!(response.message_type(), P9_RFLUSHF);
        p9_decode_rflushf(&response).unwrap();

        let response = server.handle_frame(&p9_tflushf(6, 99)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn walkgetattr_is_gated_by_google_2_and_new_fid_is_usable() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L_GOOGLE_1).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        let response = server
            .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);
        let response = server
            .handle_frame(&p9_twalkgetattr(3, 1, 2, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EOPNOTSUPP);

        let response = server
            .handle_frame(&p9_tversion(4, 8192, P9_VERSION_9P2000_L_GOOGLE_2).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        let response = server
            .handle_frame(&p9_tattach(5, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);

        let response = server
            .handle_frame(&p9_twalkgetattr(6, 1, 2, &["hello.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALKGETATTR);
        let walk = p9_decode_rwalkgetattr(&response).unwrap();
        assert_eq!(walk.valid, u64::MAX);
        assert_eq!(walk.qids.len(), 1);
        assert_eq!(walk.attr.size, 5);
        assert_eq!(walk.attr.mode & P9_MODE_TYPE_MASK, P9_MODE_REG);

        let response = server.handle_frame(&p9_tlopen(7, 2, 0)).unwrap();
        assert_eq!(response.message_type(), P9_RLOPEN);
        let response = server.handle_frame(&p9_tread(8, 2, 0, 5)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"hello");
    }

    #[test]
    fn walkgetattr_reports_virtual_owner_attrs() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("owned.txt", b"owned").unwrap();
        let mut server = server(fs);

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L_GOOGLE_2).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        let response = server
            .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);
        walk(&mut server, 1, 2, &["owned.txt"]);

        let owner = P9SetAttr {
            uid: 42,
            gid: 43,
            ..P9SetAttr::default()
        };
        let response = server
            .handle_frame(&p9_tsetattr(3, 2, P9_SETATTR_UID | P9_SETATTR_GID, &owner))
            .unwrap();
        assert_eq!(response.message_type(), P9_RSETATTR);

        let response = server
            .handle_frame(&p9_twalkgetattr(4, 1, 3, &["owned.txt"]).unwrap())
            .unwrap();
        let walk = p9_decode_rwalkgetattr(&response).unwrap();
        assert_eq!(walk.attr.uid, 42);
        assert_eq!(walk.attr.gid, 43);
    }

    #[test]
    fn walkgetattr_missing_path_does_not_install_new_fid() {
        let mut server = server(Arc::new(MemFs::new()));

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L_GOOGLE_2).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        let response = server
            .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);

        let response = server
            .handle_frame(&p9_twalkgetattr(3, 1, 2, &["missing"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, 2);

        let response = server.handle_frame(&p9_tgetattr(4, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RLERROR);
        assert_eq!(p9_decode_rlerror(&response).unwrap().ecode, EBADF);
    }

    #[test]
    fn zero_name_walkgetattr_clones_fid_path_and_returns_attrs() {
        let fs = Arc::new(MemFs::new());
        fs.write_file("hello.txt", b"hello").unwrap();
        let mut server = server(fs);

        let response = server
            .handle_frame(&p9_tversion(1, 8192, P9_VERSION_9P2000_L_GOOGLE_2).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RVERSION);
        let response = server
            .handle_frame(&p9_tattach(2, 1, 0xffff_ffff, "root", "", 0).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RATTACH);
        walk(&mut server, 1, 2, &["hello.txt"]);

        let response = server
            .handle_frame(&p9_twalkgetattr(3, 2, 3, &[]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALKGETATTR);
        let walk = p9_decode_rwalkgetattr(&response).unwrap();
        assert!(walk.qids.is_empty());
        assert_eq!(walk.attr.size, 5);

        let response = server.handle_frame(&p9_tlopen(4, 3, 0)).unwrap();
        assert_eq!(response.message_type(), P9_RLOPEN);
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

    #[derive(Debug)]
    struct StreamFs {
        queue: Arc<Mutex<VecDeque<u8>>>,
    }

    impl StreamFs {
        fn new(data: &[u8]) -> Self {
            Self {
                queue: Arc::new(Mutex::new(data.iter().copied().collect())),
            }
        }
    }

    impl FileSystem for StreamFs {
        fn open(&self, path: &NormalizedPath, _options: OpenOptions) -> FsResult<Box<dyn File>> {
            if path.as_str() != "stream" {
                return Err(FsError::NotFound);
            }
            Ok(Box::new(StreamFile {
                queue: Arc::clone(&self.queue),
            }))
        }

        fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
            match path.as_str() {
                "." => Ok(Metadata::new(FileType::Directory, 2, 0o755)),
                "stream" => Ok(Metadata::new(FileType::File, 0, 0o666)),
                _ => Err(FsError::NotFound),
            }
        }

        fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
            if path.as_str() != "." {
                return Err(FsError::NotDirectory);
            }
            Ok(vec![DirEntry::new(
                "stream",
                Metadata::new(FileType::File, 0, 0o666),
            )])
        }
    }

    #[derive(Debug)]
    struct StreamFile {
        queue: Arc<Mutex<VecDeque<u8>>>,
    }

    impl File for StreamFile {
        fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
            let mut queue = self
                .queue
                .lock()
                .map_err(|_| FsError::Other("stream lock poisoned".to_owned()))?;
            let len = queue.len().min(buf.len());
            for slot in buf.iter_mut().take(len) {
                *slot = queue.pop_front().expect("queue contains len bytes");
            }
            Ok(len)
        }

        fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
            let mut queue = self
                .queue
                .lock()
                .map_err(|_| FsError::Other("stream lock poisoned".to_owned()))?;
            queue.extend(buf);
            Ok(buf.len())
        }

        fn metadata(&self) -> FsResult<Metadata> {
            Ok(Metadata::new(FileType::File, 0, 0o666))
        }
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
