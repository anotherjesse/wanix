use wanix_protocol::{
    P9AttrBody, P9DirEntry, P9Frame, p9_decode_tgetattr, p9_decode_treaddir, p9_decode_treadlink,
    p9_decode_twalk, p9_decode_twalkgetattr, p9_decode_txattrcreate, p9_decode_txattrwalk,
    p9_dir_entry_encoded_len, p9_rgetattr, p9_rlerror, p9_rreaddir, p9_rreadlink, p9_rwalk,
    p9_rwalkgetattr,
};

use crate::attrs::{attr_for_metadata, dirent_type_for_metadata, qid_for_metadata};
use crate::path::join_walk_component;
use crate::session::P9_GOOGLE_TWALKGETATTR_VERSION;
use crate::{
    EBADF, EINVAL, EOPNOTSUPP, FidEntry, P9Server, RREADDIR_HEADER_LEN, Wanix9pError, errno_for_fs,
};

impl P9Server {
    pub(super) fn handle_walk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
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

    pub(super) fn handle_walkgetattr(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
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

    pub(super) fn handle_readlink(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
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

    pub(super) fn handle_getattr(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
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

    pub(super) fn handle_xattrwalk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let xattr = p9_decode_txattrwalk(frame)?;
        if self.fids.contains_key(&xattr.fid) {
            Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    pub(super) fn handle_xattrcreate(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let xattr = p9_decode_txattrcreate(frame)?;
        if self.fids.contains_key(&xattr.fid) {
            Ok(p9_rlerror(frame.tag(), EOPNOTSUPP))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    pub(super) fn handle_readdir(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
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
}
