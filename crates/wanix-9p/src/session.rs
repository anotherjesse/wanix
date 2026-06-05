use wanix_fs::NormalizedPath;
use wanix_protocol::{
    P9_VERSION_9P2000_L, P9_VERSION_9P2000_L_GOOGLE_1, P9_VERSION_9P2000_L_GOOGLE_2, P9Frame,
    P9Lock, P9Version, p9_decode_tattach, p9_decode_tauth, p9_decode_tflush, p9_decode_tflushf,
    p9_decode_tfsync, p9_decode_tgetlock, p9_decode_tlock, p9_decode_tstatfs, p9_decode_tversion,
    p9_rattach, p9_rflush, p9_rflushf, p9_rfsync, p9_rgetlock, p9_rlerror, p9_rlock, p9_rstatfs,
    p9_rversion,
};

use crate::{
    EBADF, ENOSYS, EOPNOTSUPP, FidEntry, P9_LOCK_STATUS_OK, P9_LOCK_TYPE_UNLOCK, P9Server,
    Wanix9pError, errno_for_fs, fs_stat,
};

const P9_GOOGLE_VERSION_PREFIX: &str = "9P2000.L.Google.";
const P9_GOOGLE_TFLUSHF_VERSION: u32 = 1;
pub(super) const P9_GOOGLE_TWALKGETATTR_VERSION: u32 = 2;

impl P9Server {
    pub(super) fn handle_version(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let P9Version { msize, version } = p9_decode_tversion(frame)?;
        self.fids.clear();
        self.owners.clear();
        self.msize = msize.min(self.max_msize);
        let (version, google_version) = negotiate_p9_version(&version);
        self.google_version = google_version;
        Ok(p9_rversion(frame.tag(), self.msize, version)?)
    }

    pub(super) fn handle_attach(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let attach = p9_decode_tattach(frame)?;
        let path = root_path()?;
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

    pub(super) fn handle_auth(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        p9_decode_tauth(frame)?;
        Ok(p9_rlerror(frame.tag(), ENOSYS))
    }

    pub(super) fn handle_flush(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        p9_decode_tflush(frame)?;
        Ok(p9_rflush(frame.tag()))
    }

    pub(super) fn handle_flushf(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
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

    pub(super) fn handle_statfs(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let statfs = p9_decode_tstatfs(frame)?;
        let Some(path) = self.fids.get(&statfs.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if let Err(error) = self.root.metadata(&path) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        Ok(p9_rstatfs(frame.tag(), fs_stat()))
    }

    pub(super) fn handle_fsync(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let fsync = p9_decode_tfsync(frame)?;
        if self.fids.contains_key(&fsync.fid) {
            Ok(p9_rfsync(frame.tag()))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    pub(super) fn handle_lock(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let lock = p9_decode_tlock(frame)?;
        if self.fids.contains_key(&lock.fid) {
            Ok(p9_rlock(frame.tag(), P9_LOCK_STATUS_OK))
        } else {
            Ok(p9_rlerror(frame.tag(), EBADF))
        }
    }

    pub(super) fn handle_getlock(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
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
}

fn root_path() -> Result<NormalizedPath, Wanix9pError> {
    NormalizedPath::new(".").map_err(|_| Wanix9pError::InvalidPath(".".to_owned()))
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
