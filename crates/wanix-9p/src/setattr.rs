use std::time::{SystemTime, UNIX_EPOCH};

use wanix_fs::{FsError, NormalizedPath, OpenOptions};
use wanix_protocol::{
    P9_SETATTR_ATIME, P9_SETATTR_ATIME_NOT_SYSTEM_TIME, P9_SETATTR_MTIME,
    P9_SETATTR_MTIME_NOT_SYSTEM_TIME, P9_SETATTR_PERMISSIONS, P9_SETATTR_SIZE, P9Frame, P9SetAttr,
    p9_decode_tsetattr, p9_rlerror, p9_rsetattr,
};

use crate::{
    EBADF, EINVAL, EOPNOTSUPP, NANOSECONDS_PER_SECOND, P9_SETATTR_KNOWN_MASK,
    P9_SETATTR_OWNER_MASK, P9_SETATTR_UNSUPPORTED_MASK, P9Server, Wanix9pError, errno_for_fs,
};

impl P9Server {
    pub(super) fn handle_setattr(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
        let setattr = p9_decode_tsetattr(frame)?;
        let Some(path) = self.fids.get(&setattr.fid).map(|entry| entry.path.clone()) else {
            return Ok(p9_rlerror(frame.tag(), EBADF));
        };
        if let Err(ecode) = validate_setattr_request(setattr.valid, &setattr.attr) {
            return Ok(p9_rlerror(frame.tag(), ecode));
        }
        if let Err(error) = self.ensure_setattr_target(&path, setattr.valid) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        if let Err(error) = self.apply_setattr_changes(&path, setattr.valid, &setattr.attr) {
            return Ok(p9_rlerror(frame.tag(), errno_for_fs(&error)));
        }
        Ok(p9_rsetattr(frame.tag()))
    }

    fn ensure_setattr_target(&self, path: &NormalizedPath, valid: u32) -> Result<(), FsError> {
        if valid & P9_SETATTR_OWNER_MASK != 0 {
            self.metadata_no_follow(path)?;
        }
        Ok(())
    }

    fn apply_setattr_changes(
        &mut self,
        path: &NormalizedPath,
        valid: u32,
        attr: &P9SetAttr,
    ) -> Result<(), FsError> {
        self.apply_setattr_size(path, valid, attr)?;
        self.apply_setattr_times(path, valid, attr)?;
        self.apply_setattr_permissions(path, valid, attr)?;
        self.apply_setattr_owner(path, valid, attr);
        Ok(())
    }

    fn apply_setattr_size(
        &self,
        path: &NormalizedPath,
        valid: u32,
        attr: &P9SetAttr,
    ) -> Result<(), FsError> {
        if valid & P9_SETATTR_SIZE == 0 {
            return Ok(());
        }
        self.set_file_size(path, attr.size)
    }

    fn apply_setattr_times(
        &self,
        path: &NormalizedPath,
        valid: u32,
        attr: &P9SetAttr,
    ) -> Result<(), FsError> {
        if valid & (P9_SETATTR_ATIME | P9_SETATTR_MTIME) == 0 {
            return Ok(());
        }
        self.set_file_times(path, valid, attr)
    }

    fn apply_setattr_permissions(
        &self,
        path: &NormalizedPath,
        valid: u32,
        attr: &P9SetAttr,
    ) -> Result<(), FsError> {
        if valid & P9_SETATTR_PERMISSIONS == 0 {
            return Ok(());
        }
        self.root.set_permissions(path, attr.permissions)
    }

    fn apply_setattr_owner(&mut self, path: &NormalizedPath, valid: u32, attr: &P9SetAttr) {
        if valid & P9_SETATTR_OWNER_MASK != 0 {
            self.set_owner_attrs(path, valid, attr);
        }
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
        let now = requested_system_time_ns(valid)?;
        let accessed_time_ns = selected_setattr_time_ns(
            valid,
            P9_SETATTR_ATIME,
            P9_SETATTR_ATIME_NOT_SYSTEM_TIME,
            metadata.accessed_time_ns(),
            (attr.atime_seconds, attr.atime_nanoseconds),
            now,
        )?;
        let modified_time_ns = selected_setattr_time_ns(
            valid,
            P9_SETATTR_MTIME,
            P9_SETATTR_MTIME_NOT_SYSTEM_TIME,
            metadata.modified_time_ns(),
            (attr.mtime_seconds, attr.mtime_nanoseconds),
            now,
        )?;
        self.root
            .set_times(path, accessed_time_ns, modified_time_ns)
    }
}

fn selected_setattr_time_ns(
    valid: u32,
    requested_bit: u32,
    explicit_time_bit: u32,
    current_time_ns: u64,
    explicit_time: (u64, u64),
    system_time_ns: Option<u64>,
) -> Result<u64, FsError> {
    if valid & requested_bit == 0 {
        return Ok(current_time_ns);
    }
    if valid & explicit_time_bit != 0 {
        return unix_time_ns(explicit_time.0, explicit_time.1);
    }
    Ok(system_time_ns.expect("system time is present when requested"))
}

fn validate_setattr_request(valid: u32, attr: &P9SetAttr) -> Result<(), u32> {
    if valid & !P9_SETATTR_KNOWN_MASK != 0 {
        return Err(EINVAL);
    }
    if valid & P9_SETATTR_UNSUPPORTED_MASK != 0 {
        return Err(EOPNOTSUPP);
    }
    validate_setattr_times(valid, attr).map_err(|error| errno_for_fs(&error))
}

fn validate_setattr_times(valid: u32, attr: &P9SetAttr) -> Result<(), FsError> {
    validate_explicit_setattr_time(
        valid,
        P9_SETATTR_ATIME,
        P9_SETATTR_ATIME_NOT_SYSTEM_TIME,
        attr.atime_seconds,
        attr.atime_nanoseconds,
    )?;
    validate_explicit_setattr_time(
        valid,
        P9_SETATTR_MTIME,
        P9_SETATTR_MTIME_NOT_SYSTEM_TIME,
        attr.mtime_seconds,
        attr.mtime_nanoseconds,
    )
}

fn validate_explicit_setattr_time(
    valid: u32,
    requested_bit: u32,
    explicit_time_bit: u32,
    seconds: u64,
    nanoseconds: u64,
) -> Result<(), FsError> {
    if valid & requested_bit != 0 && valid & explicit_time_bit != 0 {
        unix_time_ns(seconds, nanoseconds)?;
    }
    Ok(())
}

fn requested_system_time_ns(valid: u32) -> Result<Option<u64>, FsError> {
    if requests_system_time(valid) {
        return current_unix_time_ns().map(Some);
    }
    Ok(None)
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
