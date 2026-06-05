use super::codec::{
    PayloadCursor, expect_message_type, push_attr_body, push_qid, push_string, push_u16, push_u32,
    push_u64,
};
use super::{
    P9_RWALK, P9_RWALKGETATTR, P9_TWALK, P9_TWALKGETATTR, P9AttrBody, P9Error, P9Frame, P9Qid,
    P9Walk, P9WalkGetAttrResponse,
};

const P9_U8_FIELD_LEN: usize = 1;
const P9_U16_FIELD_LEN: usize = 2;
const P9_U32_FIELD_LEN: usize = 4;
const P9_U64_FIELD_LEN: usize = 8;
const P9_QID_FIELD_LEN: usize = P9_U8_FIELD_LEN + P9_U32_FIELD_LEN + P9_U64_FIELD_LEN;
const P9_ATTR_BODY_LEN: usize = 3 * P9_U32_FIELD_LEN + 15 * P9_U64_FIELD_LEN;
const P9_QID_LIST_PREFIX_LEN: usize = P9_U16_FIELD_LEN;
const P9_RWALKGETATTR_FIXED_LEN: usize =
    P9_U64_FIELD_LEN + P9_ATTR_BODY_LEN + P9_QID_LIST_PREFIX_LEN;

/// Builds a `Twalk` frame.
///
/// # Errors
///
/// Returns an error when too many names are supplied or a name cannot fit in a
/// 9P string length.
pub fn p9_twalk(tag: u16, fid: u32, newfid: u32, names: &[&str]) -> Result<P9Frame, P9Error> {
    build_walk_frame(P9_TWALK, tag, fid, newfid, names)
}

/// Builds a `Twalkgetattr` frame.
///
/// # Errors
///
/// Returns an error when too many names are supplied or a name cannot fit in a
/// 9P string length.
pub fn p9_twalkgetattr(
    tag: u16,
    fid: u32,
    newfid: u32,
    names: &[&str],
) -> Result<P9Frame, P9Error> {
    build_walk_frame(P9_TWALKGETATTR, tag, fid, newfid, names)
}

fn build_walk_frame(
    message_type: u8,
    tag: u16,
    fid: u32,
    newfid: u32,
    names: &[&str],
) -> Result<P9Frame, P9Error> {
    let name_count =
        u16::try_from(names.len()).map_err(|_| P9Error::TooManyWalkNames { count: names.len() })?;
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, newfid);
    push_u16(&mut payload, name_count);
    for name in names {
        push_string(&mut payload, name)?;
    }
    Ok(P9Frame::new(message_type, tag, payload))
}

/// Builds an `Rwalk` frame.
///
/// # Errors
///
/// Returns an error when too many QIDs are supplied.
pub fn p9_rwalk(tag: u16, qids: &[P9Qid]) -> Result<P9Frame, P9Error> {
    let qid_count =
        u16::try_from(qids.len()).map_err(|_| P9Error::TooManyWalkNames { count: qids.len() })?;
    let mut payload = Vec::with_capacity(P9_QID_LIST_PREFIX_LEN + qids.len() * P9_QID_FIELD_LEN);
    push_u16(&mut payload, qid_count);
    for qid in qids {
        push_qid(&mut payload, *qid);
    }
    Ok(P9Frame::new(P9_RWALK, tag, payload))
}

/// Builds an `Rwalkgetattr` frame.
///
/// # Errors
///
/// Returns an error when too many QIDs are supplied.
pub fn p9_rwalkgetattr(
    tag: u16,
    valid: u64,
    attr: &P9AttrBody,
    qids: &[P9Qid],
) -> Result<P9Frame, P9Error> {
    let qid_count =
        u16::try_from(qids.len()).map_err(|_| P9Error::TooManyWalkNames { count: qids.len() })?;
    let mut payload = Vec::with_capacity(P9_RWALKGETATTR_FIXED_LEN + qids.len() * P9_QID_FIELD_LEN);
    push_u64(&mut payload, valid);
    push_attr_body(&mut payload, attr);
    push_u16(&mut payload, qid_count);
    for qid in qids {
        push_qid(&mut payload, *qid);
    }
    Ok(P9Frame::new(P9_RWALKGETATTR, tag, payload))
}

/// Decodes a `Twalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Twalk` or the payload is
/// malformed.
pub fn p9_decode_twalk(frame: &P9Frame) -> Result<P9Walk, P9Error> {
    decode_walk_frame(frame, P9_TWALK)
}

/// Decodes a `Twalkgetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Twalkgetattr` or the payload is
/// malformed.
pub fn p9_decode_twalkgetattr(frame: &P9Frame) -> Result<P9Walk, P9Error> {
    decode_walk_frame(frame, P9_TWALKGETATTR)
}

fn decode_walk_frame(frame: &P9Frame, expected_message_type: u8) -> Result<P9Walk, P9Error> {
    expect_message_type(frame, expected_message_type)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let walk = read_walk_payload(&mut cursor)?;
    cursor.finish()?;
    Ok(walk)
}

fn read_walk_payload(cursor: &mut PayloadCursor<'_>) -> Result<P9Walk, P9Error> {
    let fid = cursor.read_u32()?;
    let newfid = cursor.read_u32()?;
    let names = read_walk_names(cursor)?;
    Ok(P9Walk { fid, newfid, names })
}

fn read_walk_names(cursor: &mut PayloadCursor<'_>) -> Result<Vec<String>, P9Error> {
    let name_count = cursor.read_u16()? as usize;
    let mut names = Vec::with_capacity(name_count);
    for _ in 0..name_count {
        names.push(cursor.read_string()?);
    }
    Ok(names)
}

/// Decodes an `Rwalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rwalk` or the payload is
/// malformed.
pub fn p9_decode_rwalk(frame: &P9Frame) -> Result<Vec<P9Qid>, P9Error> {
    expect_message_type(frame, P9_RWALK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let qids = read_qids_from_cursor(&mut cursor)?;
    cursor.finish()?;
    Ok(qids)
}

/// Decodes an `Rwalkgetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rwalkgetattr` or the payload is
/// malformed.
pub fn p9_decode_rwalkgetattr(frame: &P9Frame) -> Result<P9WalkGetAttrResponse, P9Error> {
    expect_message_type(frame, P9_RWALKGETATTR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let response = read_walkgetattr_response(&mut cursor)?;
    cursor.finish()?;
    Ok(response)
}

fn read_walkgetattr_response(
    cursor: &mut PayloadCursor<'_>,
) -> Result<P9WalkGetAttrResponse, P9Error> {
    let valid = cursor.read_u64()?;
    let attr = cursor.read_attr_body()?;
    let qids = read_qids_from_cursor(cursor)?;
    Ok(P9WalkGetAttrResponse { valid, attr, qids })
}

fn read_qids_from_cursor(cursor: &mut PayloadCursor<'_>) -> Result<Vec<P9Qid>, P9Error> {
    let qid_count = cursor.read_u16()? as usize;
    let mut qids = Vec::with_capacity(qid_count);
    for _ in 0..qid_count {
        qids.push(cursor.read_qid()?);
    }
    Ok(qids)
}
