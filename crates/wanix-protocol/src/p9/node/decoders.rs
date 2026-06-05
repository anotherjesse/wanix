use super::super::codec::{PayloadCursor, decode_qid_frame, expect_message_type};
use super::super::{
    P9_RLCREATE, P9_RLOPEN, P9_RMKNOD, P9_RREADLINK, P9_RSYMLINK, P9_TLCREATE, P9_TLOPEN,
    P9_TMKNOD, P9_TREADLINK, P9_TSYMLINK, P9Create, P9Error, P9Frame, P9Mknod, P9Open, P9Qid,
    P9ReadLink, P9Symlink,
};

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
