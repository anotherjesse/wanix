use wanix_fs::NormalizedPath;
use wanix_protocol::{
    P9Attr, P9AttrBody, P9Frame, P9Qid, p9_decode_tgetattr, p9_decode_treadlink, p9_decode_twalk,
    p9_decode_twalkgetattr, p9_decode_txattrcreate, p9_decode_txattrwalk, p9_rgetattr, p9_rlerror,
    p9_rreadlink, p9_rwalk, p9_rwalkgetattr,
};

use crate::attrs::attr_for_metadata;
use crate::path::join_walk_component;
use crate::session::P9_GOOGLE_TWALKGETATTR_VERSION;
use crate::{EBADF, EINVAL, EOPNOTSUPP, FidEntry, P9Server, Wanix9pError, errno_for_fs};

mod readdir;

struct WalkedPath {
    path: NormalizedPath,
    qids: Vec<P9Qid>,
}

struct WalkGetAttrPath {
    path: NormalizedPath,
    qids: Vec<P9Qid>,
    attr: P9Attr,
}

impl P9Server {
    pub(super) fn handle_walk(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let walk = p9_decode_twalk(frame)?;
        let walked = match self.resolve_walk(frame.tag(), walk.fid, &walk.names)? {
            Ok(walked) => walked,
            Err(response) => return Ok(response),
        };
        self.fids.insert(
            walk.newfid,
            FidEntry {
                path: walked.path,
                file: None,
                append: false,
            },
        );
        Ok(p9_rwalk(frame.tag(), &walked.qids)?)
    }

    pub(super) fn handle_walkgetattr(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let walk = p9_decode_twalkgetattr(frame)?;
        let walked = match self.resolve_walkgetattr(frame.tag(), walk.fid, &walk.names)? {
            Ok(walked) => walked,
            Err(response) => return Ok(response),
        };
        self.fids.insert(
            walk.newfid,
            FidEntry {
                path: walked.path.clone(),
                file: None,
                append: false,
            },
        );
        Ok(p9_rwalkgetattr(
            frame.tag(),
            walked.attr.valid,
            &P9AttrBody::from(&walked.attr),
            &walked.qids,
        )?)
    }

    fn resolve_walkgetattr(
        &self,
        tag: u16,
        fid: u32,
        names: &[String],
    ) -> Result<Result<WalkGetAttrPath, P9Frame>, Wanix9pError> {
        if self.google_version < P9_GOOGLE_TWALKGETATTR_VERSION {
            return Ok(Err(p9_rlerror(tag, EOPNOTSUPP)));
        }
        let walked = match self.resolve_walk(tag, fid, names)? {
            Ok(walked) => walked,
            Err(response) => return Ok(Err(response)),
        };
        let attr = match self.walkgetattr_attr(tag, &walked.path) {
            Ok(attr) => attr,
            Err(response) => return Ok(Err(response)),
        };
        Ok(Ok(WalkGetAttrPath {
            path: walked.path,
            qids: walked.qids,
            attr,
        }))
    }

    fn walkgetattr_attr(&self, tag: u16, path: &NormalizedPath) -> Result<P9Attr, P9Frame> {
        let metadata = match self.metadata_no_follow(path) {
            Ok(metadata) => metadata,
            Err(error) => return Err(p9_rlerror(tag, errno_for_fs(&error))),
        };
        Ok(attr_for_metadata(
            path,
            metadata,
            u64::MAX,
            self.owner_attrs(path),
        ))
    }

    fn fid_path_or_reply(&self, tag: u16, fid: u32) -> Result<NormalizedPath, P9Frame> {
        self.fids
            .get(&fid)
            .map(|entry| entry.path.clone())
            .ok_or_else(|| p9_rlerror(tag, EBADF))
    }

    fn resolve_walk(
        &self,
        tag: u16,
        fid: u32,
        names: &[String],
    ) -> Result<Result<WalkedPath, P9Frame>, Wanix9pError> {
        let Some(source_path) = self.fids.get(&fid).map(|entry| entry.path.clone()) else {
            return Ok(Err(p9_rlerror(tag, EBADF)));
        };
        let mut path = source_path;
        let mut qids = Vec::with_capacity(names.len());
        for name in names {
            path = join_walk_component(&path, name)?;
            match self.qid_for_path(&path) {
                Ok(qid) => qids.push(qid),
                Err(error) => return Ok(Err(p9_rlerror(tag, errno_for_fs(&error)))),
            }
        }
        Ok(Ok(WalkedPath { path, qids }))
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
}
