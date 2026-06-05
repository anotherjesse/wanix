use super::codec::{
    PayloadCursor, decode_qid_frame, expect_message_type, push_qid, push_string, push_u32,
};
use super::{
    P9_RLCREATE, P9_RLOPEN, P9_RMKNOD, P9_RREADLINK, P9_RSYMLINK, P9_TLCREATE, P9_TLOPEN,
    P9_TMKNOD, P9_TREADLINK, P9_TSYMLINK, P9Create, P9Error, P9Frame, P9Mknod, P9Open, P9Qid,
    P9ReadLink, P9Symlink,
};

const P9_U8_FIELD_LEN: usize = 1;
const P9_U32_FIELD_LEN: usize = 4;
const P9_U64_FIELD_LEN: usize = 8;
const P9_QID_FIELD_LEN: usize = P9_U8_FIELD_LEN + P9_U32_FIELD_LEN + P9_U64_FIELD_LEN;
const P9_TLOPEN_PAYLOAD_LEN: usize = P9_U32_FIELD_LEN + P9_U32_FIELD_LEN;
const P9_OPEN_RESPONSE_PAYLOAD_LEN: usize = P9_QID_FIELD_LEN + P9_U32_FIELD_LEN;

/// Builds a `Tlopen` frame.
#[must_use]
pub fn p9_tlopen(tag: u16, fid: u32, flags: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_TLOPEN_PAYLOAD_LEN);
    push_u32(&mut payload, fid);
    push_u32(&mut payload, flags);
    P9Frame::new(P9_TLOPEN, tag, payload)
}

/// Builds an `Rlopen` frame.
#[must_use]
pub fn p9_rlopen(tag: u16, qid: P9Qid, iounit: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_OPEN_RESPONSE_PAYLOAD_LEN);
    push_qid(&mut payload, qid);
    push_u32(&mut payload, iounit);
    P9Frame::new(P9_RLOPEN, tag, payload)
}

/// Builds a `Tlcreate` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tlcreate(
    tag: u16,
    fid: u32,
    name: &str,
    flags: u32,
    mode: u32,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, flags);
    push_u32(&mut payload, mode);
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TLCREATE, tag, payload))
}

/// Builds an `Rlcreate` frame.
#[must_use]
pub fn p9_rlcreate(tag: u16, qid: P9Qid, iounit: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_OPEN_RESPONSE_PAYLOAD_LEN);
    push_qid(&mut payload, qid);
    push_u32(&mut payload, iounit);
    P9Frame::new(P9_RLCREATE, tag, payload)
}

/// Builds a `Tsymlink` frame.
///
/// # Errors
///
/// Returns an error when either string cannot fit in a 9P string field.
pub fn p9_tsymlink(
    tag: u16,
    dir_fid: u32,
    name: &str,
    target: &str,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_string(&mut payload, target)?;
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TSYMLINK, tag, payload))
}

/// Builds an `Rsymlink` frame.
#[must_use]
pub fn p9_rsymlink(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_QID_FIELD_LEN);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RSYMLINK, tag, payload)
}

/// Builds a `Tmknod` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tmknod(
    tag: u16,
    dir_fid: u32,
    name: &str,
    mode: u32,
    major: u32,
    minor: u32,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, mode);
    push_u32(&mut payload, major);
    push_u32(&mut payload, minor);
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TMKNOD, tag, payload))
}

/// Builds an `Rmknod` frame.
#[must_use]
pub fn p9_rmknod(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_QID_FIELD_LEN);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RMKNOD, tag, payload)
}

/// Builds a `Treadlink` frame.
#[must_use]
pub fn p9_treadlink(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_U32_FIELD_LEN);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TREADLINK, tag, payload)
}

/// Builds an `Rreadlink` frame.
///
/// # Errors
///
/// Returns an error when the target cannot fit in a 9P string field.
pub fn p9_rreadlink(tag: u16, target: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_string(&mut payload, target)?;
    Ok(P9Frame::new(P9_RREADLINK, tag, payload))
}

/// Decodes a `Tlopen` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlopen` or the payload is
/// malformed.
pub fn p9_decode_tlopen(frame: &P9Frame) -> Result<P9Open, P9Error> {
    expect_message_type(frame, P9_TLOPEN)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let flags = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Open { fid, flags })
}

/// Decodes an `Rlopen` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlopen` or the payload is
/// malformed.
pub fn p9_decode_rlopen(frame: &P9Frame) -> Result<(P9Qid, u32), P9Error> {
    expect_message_type(frame, P9_RLOPEN)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qid = cursor.read_qid()?;
    let iounit = cursor.read_u32()?;
    cursor.finish()?;
    Ok((qid, iounit))
}

/// Decodes a `Tlcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlcreate` or the payload is
/// malformed.
pub fn p9_decode_tlcreate(frame: &P9Frame) -> Result<P9Create, P9Error> {
    expect_message_type(frame, P9_TLCREATE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let flags = cursor.read_u32()?;
    let mode = cursor.read_u32()?;
    let gid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Create {
        fid,
        name,
        flags,
        mode,
        gid,
    })
}

/// Decodes an `Rlcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlcreate` or the payload is
/// malformed.
pub fn p9_decode_rlcreate(frame: &P9Frame) -> Result<(P9Qid, u32), P9Error> {
    expect_message_type(frame, P9_RLCREATE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qid = cursor.read_qid()?;
    let iounit = cursor.read_u32()?;
    cursor.finish()?;
    Ok((qid, iounit))
}

/// Decodes a `Tsymlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tsymlink` or the payload is
/// malformed.
pub fn p9_decode_tsymlink(frame: &P9Frame) -> Result<P9Symlink, P9Error> {
    expect_message_type(frame, P9_TSYMLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let target = cursor.read_string()?;
    let gid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Symlink {
        dir_fid,
        name,
        target,
        gid,
    })
}

/// Decodes an `Rsymlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rsymlink` or the payload is
/// malformed.
pub fn p9_decode_rsymlink(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RSYMLINK)
}

/// Decodes a `Tmknod` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tmknod` or the payload is
/// malformed.
pub fn p9_decode_tmknod(frame: &P9Frame) -> Result<P9Mknod, P9Error> {
    expect_message_type(frame, P9_TMKNOD)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let mode = cursor.read_u32()?;
    let major = cursor.read_u32()?;
    let minor = cursor.read_u32()?;
    let gid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Mknod {
        dir_fid,
        name,
        mode,
        major,
        minor,
        gid,
    })
}

/// Decodes an `Rmknod` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rmknod` or the payload is
/// malformed.
pub fn p9_decode_rmknod(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RMKNOD)
}

/// Decodes a `Treadlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Treadlink` or the payload is
/// malformed.
pub fn p9_decode_treadlink(frame: &P9Frame) -> Result<P9ReadLink, P9Error> {
    expect_message_type(frame, P9_TREADLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9ReadLink { fid })
}

/// Decodes an `Rreadlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rreadlink` or the payload is
/// malformed.
pub fn p9_decode_rreadlink(frame: &P9Frame) -> Result<String, P9Error> {
    expect_message_type(frame, P9_RREADLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let target = cursor.read_string()?;
    cursor.finish()?;
    Ok(target)
}
