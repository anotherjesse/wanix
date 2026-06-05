use std::time::{SystemTime, UNIX_EPOCH};

use wanix_fs::FsError;
use wanix_protocol::{
    P9_SETATTR_ATIME, P9_SETATTR_ATIME_NOT_SYSTEM_TIME, P9_SETATTR_MTIME,
    P9_SETATTR_MTIME_NOT_SYSTEM_TIME,
};

use crate::NANOSECONDS_PER_SECOND;

pub(super) struct SetattrTimeSelection {
    pub(super) valid: u32,
    pub(super) requested_bit: u32,
    pub(super) explicit_time_bit: u32,
    pub(super) current_time_ns: u64,
    pub(super) explicit_time: (u64, u64),
    pub(super) system_time_ns: Option<u64>,
}

pub(super) fn selected_setattr_time_ns(selection: SetattrTimeSelection) -> Result<u64, FsError> {
    if selection.valid & selection.requested_bit == 0 {
        return Ok(selection.current_time_ns);
    }
    if selection.valid & selection.explicit_time_bit != 0 {
        return unix_time_ns(selection.explicit_time.0, selection.explicit_time.1);
    }
    selection.system_time_ns.ok_or(FsError::InvalidTime)
}

pub(super) fn requested_system_time_ns(valid: u32) -> Result<Option<u64>, FsError> {
    if requests_system_time(valid) {
        return current_unix_time_ns().map(Some);
    }
    Ok(None)
}

fn requests_system_time(valid: u32) -> bool {
    valid & P9_SETATTR_ATIME != 0 && valid & P9_SETATTR_ATIME_NOT_SYSTEM_TIME == 0
        || valid & P9_SETATTR_MTIME != 0 && valid & P9_SETATTR_MTIME_NOT_SYSTEM_TIME == 0
}

pub(super) fn unix_time_ns(seconds: u64, nanoseconds: u64) -> Result<u64, FsError> {
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
