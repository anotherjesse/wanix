use super::super::codec::{push_fs_stat, push_lock_range, push_u16, push_u32};
use super::super::{
    P9_RFLUSH, P9_RFLUSHF, P9_RFSYNC, P9_RGETLOCK, P9_RLERROR, P9_RLOCK, P9_RSTATFS, P9_TFLUSH,
    P9_TFLUSHF, P9_TFSYNC, P9_TGETLOCK, P9_TLOCK, P9_TSTATFS, P9Error, P9Frame, P9FsStat, P9Lock,
};

const P9_U16_FIELD_LEN: usize = 2;
const P9_U32_FIELD_LEN: usize = 4;
const P9_U64_FIELD_LEN: usize = 8;
const P9_RSTATFS_PAYLOAD_LEN: usize = (3 * P9_U32_FIELD_LEN) + (6 * P9_U64_FIELD_LEN);

/// Builds an `Rlerror` frame.
#[must_use]
pub fn p9_rlerror(tag: u16, ecode: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_U32_FIELD_LEN);
    push_u32(&mut payload, ecode);
    P9Frame::new(P9_RLERROR, tag, payload)
}

/// Builds a `Tstatfs` frame.
#[must_use]
pub fn p9_tstatfs(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_U32_FIELD_LEN);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TSTATFS, tag, payload)
}

/// Builds an `Rstatfs` frame.
#[must_use]
pub fn p9_rstatfs(tag: u16, stat: P9FsStat) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_RSTATFS_PAYLOAD_LEN);
    push_fs_stat(&mut payload, stat);
    P9Frame::new(P9_RSTATFS, tag, payload)
}

/// Builds a `Tfsync` frame.
#[must_use]
pub fn p9_tfsync(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_U32_FIELD_LEN);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TFSYNC, tag, payload)
}

/// Builds an `Rfsync` frame.
#[must_use]
pub fn p9_rfsync(tag: u16) -> P9Frame {
    P9Frame::new(P9_RFSYNC, tag, Vec::new())
}

/// Builds a `Tlock` frame.
///
/// # Errors
///
/// Returns an error when the client id cannot fit in a 9P string field.
pub fn p9_tlock(tag: u16, fid: u32, flags: u32, lock: &P9Lock) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    payload.push(lock.lock_type);
    push_u32(&mut payload, flags);
    push_lock_range(&mut payload, lock)?;
    Ok(P9Frame::new(P9_TLOCK, tag, payload))
}

/// Builds an `Rlock` frame.
#[must_use]
pub fn p9_rlock(tag: u16, status: u8) -> P9Frame {
    P9Frame::new(P9_RLOCK, tag, vec![status])
}

/// Builds a `Tgetlock` frame.
///
/// # Errors
///
/// Returns an error when the client id cannot fit in a 9P string field.
pub fn p9_tgetlock(tag: u16, fid: u32, lock: &P9Lock) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    payload.push(lock.lock_type);
    push_lock_range(&mut payload, lock)?;
    Ok(P9Frame::new(P9_TGETLOCK, tag, payload))
}

/// Builds an `Rgetlock` frame.
///
/// # Errors
///
/// Returns an error when the client id cannot fit in a 9P string field.
pub fn p9_rgetlock(tag: u16, lock: &P9Lock) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    payload.push(lock.lock_type);
    push_lock_range(&mut payload, lock)?;
    Ok(P9Frame::new(P9_RGETLOCK, tag, payload))
}

/// Builds a `Tflush` frame.
#[must_use]
pub fn p9_tflush(tag: u16, oldtag: u16) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_U16_FIELD_LEN);
    push_u16(&mut payload, oldtag);
    P9Frame::new(P9_TFLUSH, tag, payload)
}

/// Builds an `Rflush` frame.
#[must_use]
pub fn p9_rflush(tag: u16) -> P9Frame {
    P9Frame::new(P9_RFLUSH, tag, Vec::new())
}

/// Builds a `Tflushf` frame.
#[must_use]
pub fn p9_tflushf(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_U32_FIELD_LEN);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TFLUSHF, tag, payload)
}

/// Builds an `Rflushf` frame.
#[must_use]
pub fn p9_rflushf(tag: u16) -> P9Frame {
    P9Frame::new(P9_RFLUSHF, tag, Vec::new())
}
