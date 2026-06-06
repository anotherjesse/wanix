//! 9P server adapters backed by Rust Wanix filesystems.
//!
//! This crate maps typed `wanix-protocol` 9P frames onto `wanix-fs`
//! filesystems. It owns fid state and filesystem error mapping, while the
//! protocol crate remains dependency-free and wire-only.

mod attrs;
mod dispatch;
mod error;
mod io;
mod lookup;
mod mutation;
mod path;
mod session;
mod setattr;
mod transport;

use std::collections::BTreeMap;
use std::sync::Arc;

use wanix_fs::{File, FileSystem, FsError, Metadata, MetadataLookup, NormalizedPath, OpenOptions};
use wanix_protocol::{
    P9_SETATTR_ATIME, P9_SETATTR_ATIME_NOT_SYSTEM_TIME, P9_SETATTR_CTIME, P9_SETATTR_GID,
    P9_SETATTR_MTIME, P9_SETATTR_MTIME_NOT_SYSTEM_TIME, P9_SETATTR_PERMISSIONS, P9_SETATTR_SIZE,
    P9_SETATTR_UID, P9Frame, P9Qid, P9SetAttr, p9_decode_tclunk, p9_rclunk, p9_rlerror,
};

pub use error::Wanix9pError;
pub use transport::{P9TransportError, P9TransportStats};

pub(crate) use error::{EBADF, EINVAL, EISDIR, ENOSYS, EOPNOTSUPP, errno_for_fs};

#[cfg(test)]
pub(crate) use error::{EACCES, ENOTDIR};

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

    fn fid_path_or_reply(&self, tag: u16, fid: u32) -> Result<NormalizedPath, P9Frame> {
        self.fids
            .get(&fid)
            .map(|entry| entry.path.clone())
            .ok_or_else(|| p9_rlerror(tag, EBADF))
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

#[cfg(test)]
mod tests;
