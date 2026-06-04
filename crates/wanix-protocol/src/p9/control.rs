use super::codec::{
    PayloadCursor, expect_message_type, push_fs_stat, push_lock_range, push_u16, push_u32,
};
use super::{
    P9_RFLUSH, P9_RFLUSHF, P9_RFSYNC, P9_RGETLOCK, P9_RLERROR, P9_RLOCK, P9_RSTATFS, P9_TFLUSH,
    P9_TFLUSHF, P9_TFSYNC, P9_TGETLOCK, P9_TLOCK, P9_TSTATFS, P9Error, P9Flush, P9FlushF, P9Frame,
    P9FsStat, P9Fsync, P9GetLockRequest, P9Lerror, P9Lock, P9LockRequest, P9StatFs,
};

/// Builds an `Rlerror` frame.
#[must_use]
pub fn p9_rlerror(tag: u16, ecode: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, ecode);
    P9Frame::new(P9_RLERROR, tag, payload)
}

/// Builds a `Tstatfs` frame.
#[must_use]
pub fn p9_tstatfs(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TSTATFS, tag, payload)
}

/// Builds an `Rstatfs` frame.
#[must_use]
pub fn p9_rstatfs(tag: u16, stat: P9FsStat) -> P9Frame {
    let mut payload = Vec::with_capacity(60);
    push_fs_stat(&mut payload, stat);
    P9Frame::new(P9_RSTATFS, tag, payload)
}

/// Builds a `Tfsync` frame.
#[must_use]
pub fn p9_tfsync(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
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
    let mut payload = Vec::with_capacity(2);
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
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TFLUSHF, tag, payload)
}

/// Builds an `Rflushf` frame.
#[must_use]
pub fn p9_rflushf(tag: u16) -> P9Frame {
    P9Frame::new(P9_RFLUSHF, tag, Vec::new())
}

/// Decodes an `Rlerror` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlerror` or the payload is
/// malformed.
pub fn p9_decode_rlerror(frame: &P9Frame) -> Result<P9Lerror, P9Error> {
    expect_message_type(frame, P9_RLERROR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let ecode = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Lerror { ecode })
}

/// Decodes a `Tstatfs` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tstatfs` or the payload is
/// malformed.
pub fn p9_decode_tstatfs(frame: &P9Frame) -> Result<P9StatFs, P9Error> {
    expect_message_type(frame, P9_TSTATFS)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9StatFs { fid })
}

/// Decodes an `Rstatfs` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rstatfs` or the payload is
/// malformed.
pub fn p9_decode_rstatfs(frame: &P9Frame) -> Result<P9FsStat, P9Error> {
    expect_message_type(frame, P9_RSTATFS)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let stat = cursor.read_fs_stat()?;
    cursor.finish()?;
    Ok(stat)
}

/// Decodes a `Tfsync` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tfsync` or the payload is
/// malformed.
pub fn p9_decode_tfsync(frame: &P9Frame) -> Result<P9Fsync, P9Error> {
    expect_message_type(frame, P9_TFSYNC)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Fsync { fid })
}

/// Decodes an `Rfsync` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rfsync` or the payload is not
/// empty.
pub fn p9_decode_rfsync(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RFSYNC)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlock` or the payload is
/// malformed.
pub fn p9_decode_tlock(frame: &P9Frame) -> Result<P9LockRequest, P9Error> {
    expect_message_type(frame, P9_TLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let request = read_lock_request(&mut cursor)?;
    cursor.finish()?;
    Ok(request)
}

fn read_lock_request(cursor: &mut PayloadCursor<'_>) -> Result<P9LockRequest, P9Error> {
    let fid = cursor.read_u32()?;
    let lock_type = cursor.read_u8()?;
    let flags = cursor.read_u32()?;
    let lock = cursor.read_lock(lock_type)?;
    Ok(P9LockRequest { fid, flags, lock })
}

/// Decodes an `Rlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlock` or the payload is
/// malformed.
pub fn p9_decode_rlock(frame: &P9Frame) -> Result<u8, P9Error> {
    expect_message_type(frame, P9_RLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let status = cursor.read_u8()?;
    cursor.finish()?;
    Ok(status)
}

/// Decodes a `Tgetlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tgetlock` or the payload is
/// malformed.
pub fn p9_decode_tgetlock(frame: &P9Frame) -> Result<P9GetLockRequest, P9Error> {
    expect_message_type(frame, P9_TGETLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let request = read_getlock_request(&mut cursor)?;
    cursor.finish()?;
    Ok(request)
}

fn read_getlock_request(cursor: &mut PayloadCursor<'_>) -> Result<P9GetLockRequest, P9Error> {
    let fid = cursor.read_u32()?;
    let lock_type = cursor.read_u8()?;
    let lock = cursor.read_lock(lock_type)?;
    Ok(P9GetLockRequest { fid, lock })
}

/// Decodes an `Rgetlock` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rgetlock` or the payload is
/// malformed.
pub fn p9_decode_rgetlock(frame: &P9Frame) -> Result<P9Lock, P9Error> {
    expect_message_type(frame, P9_RGETLOCK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let lock_type = cursor.read_u8()?;
    let lock = cursor.read_lock(lock_type)?;
    cursor.finish()?;
    Ok(lock)
}

/// Decodes a `Tflush` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tflush` or the payload is
/// malformed.
pub fn p9_decode_tflush(frame: &P9Frame) -> Result<P9Flush, P9Error> {
    expect_message_type(frame, P9_TFLUSH)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let oldtag = cursor.read_u16()?;
    cursor.finish()?;
    Ok(P9Flush { oldtag })
}

/// Decodes an `Rflush` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rflush` or the payload is not
/// empty.
pub fn p9_decode_rflush(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RFLUSH)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tflushf` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tflushf` or the payload is
/// malformed.
pub fn p9_decode_tflushf(frame: &P9Frame) -> Result<P9FlushF, P9Error> {
    expect_message_type(frame, P9_TFLUSHF)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9FlushF { fid })
}

/// Decodes an `Rflushf` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rflushf` or the payload is not
/// empty.
pub fn p9_decode_rflushf(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RFLUSHF)?;
    PayloadCursor::new(frame.payload()).finish()
}
