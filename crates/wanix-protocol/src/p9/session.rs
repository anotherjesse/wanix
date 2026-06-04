use super::codec::{
    PayloadCursor, decode_qid_frame, decode_version_frame, encode_version_payload,
    expect_message_type, push_qid, push_string, push_u32,
};
use super::{
    P9_RATTACH, P9_RAUTH, P9_RVERSION, P9_TATTACH, P9_TAUTH, P9_TVERSION, P9Attach, P9Auth,
    P9Error, P9Frame, P9Qid, P9Version,
};

/// Builds a `Tversion` frame.
///
/// # Errors
///
/// Returns an error when the version string cannot fit in a 9P string field.
pub fn p9_tversion(tag: u16, msize: u32, version: &str) -> Result<P9Frame, P9Error> {
    Ok(P9Frame::new(
        P9_TVERSION,
        tag,
        encode_version_payload(msize, version)?,
    ))
}

/// Builds an `Rversion` frame.
///
/// # Errors
///
/// Returns an error when the version string cannot fit in a 9P string field.
pub fn p9_rversion(tag: u16, msize: u32, version: &str) -> Result<P9Frame, P9Error> {
    Ok(P9Frame::new(
        P9_RVERSION,
        tag,
        encode_version_payload(msize, version)?,
    ))
}

/// Decodes a `Tversion` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tversion` or the payload is
/// malformed.
pub fn p9_decode_tversion(frame: &P9Frame) -> Result<P9Version, P9Error> {
    decode_version_frame(frame, P9_TVERSION)
}

/// Decodes an `Rversion` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rversion` or the payload is
/// malformed.
pub fn p9_decode_rversion(frame: &P9Frame) -> Result<P9Version, P9Error> {
    decode_version_frame(frame, P9_RVERSION)
}

/// Builds a `Tauth` frame.
///
/// # Errors
///
/// Returns an error when a string field cannot fit in a 9P string length.
pub fn p9_tauth(
    tag: u16,
    afid: u32,
    uname: &str,
    aname: &str,
    n_uname: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, afid);
    push_string(&mut payload, uname)?;
    push_string(&mut payload, aname)?;
    push_u32(&mut payload, n_uname);
    Ok(P9Frame::new(P9_TAUTH, tag, payload))
}

/// Builds an `Rauth` frame.
#[must_use]
pub fn p9_rauth(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(13);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RAUTH, tag, payload)
}

/// Builds a `Tattach` frame.
///
/// # Errors
///
/// Returns an error when a string field cannot fit in a 9P string length.
pub fn p9_tattach(
    tag: u16,
    fid: u32,
    afid: u32,
    uname: &str,
    aname: &str,
    n_uname: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, afid);
    push_string(&mut payload, uname)?;
    push_string(&mut payload, aname)?;
    push_u32(&mut payload, n_uname);
    Ok(P9Frame::new(P9_TATTACH, tag, payload))
}

/// Builds an `Rattach` frame.
#[must_use]
pub fn p9_rattach(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(13);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RATTACH, tag, payload)
}

/// Decodes a `Tauth` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tauth` or the payload is
/// malformed.
pub fn p9_decode_tauth(frame: &P9Frame) -> Result<P9Auth, P9Error> {
    expect_message_type(frame, P9_TAUTH)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let afid = cursor.read_u32()?;
    let uname = cursor.read_string()?;
    let aname = cursor.read_string()?;
    let n_uname = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Auth {
        afid,
        uname,
        aname,
        n_uname,
    })
}

/// Decodes an `Rauth` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rauth` or the payload is
/// malformed.
pub fn p9_decode_rauth(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RAUTH)
}

/// Decodes a `Tattach` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tattach` or the payload is
/// malformed.
pub fn p9_decode_tattach(frame: &P9Frame) -> Result<P9Attach, P9Error> {
    expect_message_type(frame, P9_TATTACH)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let afid = cursor.read_u32()?;
    let uname = cursor.read_string()?;
    let aname = cursor.read_string()?;
    let n_uname = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Attach {
        fid,
        afid,
        uname,
        aname,
        n_uname,
    })
}

/// Decodes an `Rattach` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rattach` or the payload is
/// malformed.
pub fn p9_decode_rattach(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RATTACH)
}
