//! 9P server adapters backed by Rust Wanix filesystems.
//!
//! This crate maps typed `wanix-protocol` 9P frames onto `wanix-fs`
//! filesystems. It owns fid state and filesystem error mapping, while the
//! protocol crate remains dependency-free and wire-only.

mod attrs;
mod dispatch;
mod io;
mod lookup;
mod mutation;
mod path;
mod session;
mod setattr;
mod transport;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use wanix_fs::{File, FileSystem, FsError, Metadata, MetadataLookup, NormalizedPath, OpenOptions};
use wanix_protocol::{
    P9_SETATTR_ATIME, P9_SETATTR_ATIME_NOT_SYSTEM_TIME, P9_SETATTR_CTIME, P9_SETATTR_GID,
    P9_SETATTR_MTIME, P9_SETATTR_MTIME_NOT_SYSTEM_TIME, P9_SETATTR_PERMISSIONS, P9_SETATTR_SIZE,
    P9_SETATTR_UID, P9Error, P9Frame, P9Qid, P9SetAttr, p9_decode_tclunk, p9_rclunk, p9_rlerror,
};

pub use transport::{P9TransportError, P9TransportStats};

#[cfg(test)]
use attrs::{
    DT_DIR, DT_REG, P9_DEFAULT_BLOCK_SIZE, P9_DEFAULT_NAME_LENGTH, P9_FS_MAGIC, P9_MODE_DIR,
    P9_MODE_LNK, P9_MODE_REG, P9_MODE_TYPE_MASK,
};
use attrs::{fs_stat, qid_for_metadata};
use path::{is_same_or_descendant_path, rebase_path};

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

    use wanix_fs::{DirEntry, FileType, FsResult, LocalFs, MemFs};
    use wanix_protocol::{
        P9_LOCK_TYPE_READ, P9_LOCK_TYPE_WRITE, P9_RATTACH, P9_RFLUSH, P9_RFLUSHF, P9_RFSYNC,
        P9_RGETATTR, P9_RGETLOCK, P9_RLCREATE, P9_RLERROR, P9_RLINK, P9_RLOCK, P9_RLOPEN,
        P9_RMKDIR, P9_RREAD, P9_RREADDIR, P9_RREADLINK, P9_RREMOVE, P9_RRENAME, P9_RRENAMEAT,
        P9_RSETATTR, P9_RSTATFS, P9_RSYMLINK, P9_RUNLINKAT, P9_RVERSION, P9_RWALK, P9_RWALKGETATTR,
        P9_RWRITE, P9_SETATTR_ATIME, P9_SETATTR_ATIME_NOT_SYSTEM_TIME, P9_SETATTR_MTIME,
        P9_SETATTR_MTIME_NOT_SYSTEM_TIME, P9_SETATTR_PERMISSIONS, P9_SETATTR_SIZE,
        P9_VERSION_9P2000_L, P9_VERSION_9P2000_L_GOOGLE_1, P9_VERSION_9P2000_L_GOOGLE_2,
        P9DirEntry, P9Lock, P9SetAttr, p9_decode_rflush, p9_decode_rflushf, p9_decode_rfsync,
        p9_decode_rgetattr, p9_decode_rgetlock, p9_decode_rlcreate, p9_decode_rlerror,
        p9_decode_rlink, p9_decode_rlock, p9_decode_rlopen, p9_decode_rmkdir, p9_decode_rread,
        p9_decode_rreaddir, p9_decode_rreadlink, p9_decode_rremove, p9_decode_rrename,
        p9_decode_rsetattr, p9_decode_rstatfs, p9_decode_rsymlink, p9_decode_rversion,
        p9_decode_rwalk, p9_decode_rwalkgetattr, p9_decode_rwrite, p9_dir_entry_encoded_len,
        p9_tattach, p9_tauth, p9_tclunk, p9_tflush, p9_tflushf, p9_tfsync, p9_tgetattr,
        p9_tgetlock, p9_tlcreate, p9_tlink, p9_tlock, p9_tlopen, p9_tmkdir, p9_tmknod, p9_tread,
        p9_treaddir, p9_treadlink, p9_tremove, p9_trename, p9_trenameat, p9_tsetattr, p9_tstatfs,
        p9_tsymlink, p9_tunlinkat, p9_tversion, p9_twalk, p9_twalkgetattr, p9_twrite,
        p9_txattrcreate, p9_txattrwalk,
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
    #[cfg(unix)]
    fn hard_link_mutates_host_backed_filesystem() {
        let root = temp_dir("wanix-9p-hard-link");
        fs::write(root.join("target.txt"), b"linked data").unwrap();
        let local = Arc::new(LocalFs::new(&root).unwrap());
        let root_fs: Arc<dyn FileSystem> = local;
        let mut server = P9Server::new(root_fs);

        attach_root(&mut server);
        walk(&mut server, 1, 2, &["target.txt"]);
        let response = server
            .handle_frame(&p9_tlink(3, 1, 2, "hard.txt").unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RLINK);
        p9_decode_rlink(&response).unwrap();
        assert_eq!(fs::read(root.join("hard.txt")).unwrap(), b"linked data");

        let response = server.handle_frame(&p9_tgetattr(4, 2, u64::MAX)).unwrap();
        assert_eq!(response.message_type(), P9_RGETATTR);
        assert_eq!(p9_decode_rgetattr(&response).unwrap().nlink, 2);

        let response = server
            .handle_frame(&p9_twalk(5, 1, 3, &["hard.txt"]).unwrap())
            .unwrap();
        assert_eq!(response.message_type(), P9_RWALK);
        let response = server.handle_frame(&p9_tlopen(6, 3, 0)).unwrap();
        assert_eq!(response.message_type(), P9_RLOPEN);
        let response = server.handle_frame(&p9_tread(7, 3, 0, 11)).unwrap();
        assert_eq!(response.message_type(), P9_RREAD);
        assert_eq!(p9_decode_rread(&response).unwrap(), b"linked data");
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
