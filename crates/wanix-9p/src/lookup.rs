use wanix_fs::{File, FileType, FsResult, Metadata, NormalizedPath};
use wanix_protocol::{
    P9Attr, P9AttrBody, P9Frame, P9Qid, p9_decode_tgetattr, p9_decode_treadlink, p9_decode_twalk,
    p9_decode_twalkgetattr, p9_decode_txattrcreate, p9_decode_txattrwalk, p9_rgetattr, p9_rlerror,
    p9_rreadlink, p9_rwalk, p9_rwalkgetattr, p9_rxattrwalk,
};

use crate::attrs::attr_for_metadata;
use crate::path::join_walk_component;
use crate::session::P9_GOOGLE_TWALKGETATTR_VERSION;
use crate::{EBADF, EINVAL, ENODATA, EOPNOTSUPP, FidEntry, P9Server, Wanix9pError, errno_for_fs};

/// The single extended-attribute name this server synthesizes: the BLAKE3
/// content hash a CAS-aware client uses to offload bulk reads to the blob plane.
///
/// A `Txattrwalk` for this name walks to a read-only fid serving the 64
/// lowercase-hex characters of [`wanix_fs::FileSystem::content_hash`]; any other
/// name, or a file with no hash, yields `ENODATA`. This is the blueprint's
/// "genuine synthetic `File` serving 64 hex bytes" — the hash rides the wire as
/// a real xattr read, never as bytes appended to a fixed-shape `Rgetattr`.
pub(crate) const CAS_HASH_XATTR: &str = "cas.hash";

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

    fn resolve_walk(
        &self,
        tag: u16,
        fid: u32,
        names: &[String],
    ) -> Result<Result<WalkedPath, P9Frame>, Wanix9pError> {
        let source_path = match self.fid_path_or_reply(tag, fid) {
            Ok(path) => path,
            Err(response) => return Ok(Err(response)),
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
        let path = match self.fid_path_or_reply(frame.tag(), readlink.fid) {
            Ok(path) => path,
            Err(response) => return Ok(response),
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
        let path = match self.fid_path_or_reply(frame.tag(), getattr.fid) {
            Ok(path) => path,
            Err(response) => return Ok(response),
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
        let Some(path) = self.fids.get(&xattr.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        // The only attribute this server synthesizes is `cas.hash`: the BLAKE3
        // content address that lets a CAS-aware client skip the `Tread` loop and
        // fetch the blob from the data plane. Anything else is unsupported.
        if xattr.name != CAS_HASH_XATTR {
            return Ok(p9_rlerror(frame.tag(), EOPNOTSUPP));
        }
        let hash = match self.root.content_hash(&path) {
            // `Ok(None)` means "resolvable but not content-addressed" (small,
            // mid-write, or no blob backing): the attribute simply does not
            // exist, so the client falls back to a plain `Tread`.
            Ok(None) => return Ok(p9_rlerror(frame.tag(), ENODATA)),
            Ok(Some(hash)) => hash,
            Err(error) => return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error))),
        };
        // The new fid is bound to the path *and* pre-opened with a synthetic
        // reader over the 64 hex bytes, so the client reads the value with a
        // single `Tread` and no separate `Tlopen` (matching xattr semantics).
        let hex = hash.to_hex().into_bytes();
        let size = hex.len() as u64;
        self.fids.insert(
            xattr.newfid,
            FidEntry {
                path,
                file: Some(Box::new(XattrValueFile::new(hex))),
                append: false,
            },
        );
        Ok(p9_rxattrwalk(frame.tag(), size))
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

/// A read-only, in-memory [`File`] serving a fixed xattr value (the hex content
/// hash). It is pre-installed on the `Txattrwalk` fid so the value reads with a
/// single `Tread`; a follow-up read returns end-of-file.
struct XattrValueFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl XattrValueFile {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for XattrValueFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        // A streaming reader: each read advances an internal cursor, so a second
        // read past the value returns 0 (EOF). The value is at most 64 bytes.
        let remaining = self.bytes.len().saturating_sub(self.offset);
        let len = remaining.min(buf.len());
        buf[..len].copy_from_slice(&self.bytes[self.offset..self.offset + len]);
        self.offset += len;
        Ok(len)
    }

    fn is_seekable(&self) -> bool {
        false
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(
            FileType::File,
            self.bytes.len() as u64,
            0o444,
        ))
    }
}
