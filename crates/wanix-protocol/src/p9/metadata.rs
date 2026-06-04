use super::codec::{
    PayloadCursor, expect_message_type, push_attr, push_set_attr, push_string, push_u32, push_u64,
};
use super::{
    P9_RGETATTR, P9_RSETATTR, P9_RXATTRCREATE, P9_RXATTRWALK, P9_TGETATTR, P9_TSETATTR,
    P9_TXATTRCREATE, P9_TXATTRWALK, P9Attr, P9Error, P9Frame, P9GetAttr, P9SetAttr,
    P9SetAttrRequest, P9XattrCreate, P9XattrWalk,
};

/// Builds a `Tgetattr` frame.
#[must_use]
pub fn p9_tgetattr(tag: u16, fid: u32, request_mask: u64) -> P9Frame {
    let mut payload = Vec::with_capacity(12);
    push_u32(&mut payload, fid);
    push_u64(&mut payload, request_mask);
    P9Frame::new(P9_TGETATTR, tag, payload)
}

/// Builds an `Rgetattr` frame.
#[must_use]
pub fn p9_rgetattr(tag: u16, attr: &P9Attr) -> P9Frame {
    let mut payload = Vec::with_capacity(153);
    push_attr(&mut payload, attr);
    P9Frame::new(P9_RGETATTR, tag, payload)
}

/// Builds a `Tsetattr` frame.
#[must_use]
pub fn p9_tsetattr(tag: u16, fid: u32, valid: u32, attr: &P9SetAttr) -> P9Frame {
    let mut payload = Vec::with_capacity(60);
    push_u32(&mut payload, fid);
    push_u32(&mut payload, valid);
    push_set_attr(&mut payload, attr);
    P9Frame::new(P9_TSETATTR, tag, payload)
}

/// Builds an `Rsetattr` frame.
#[must_use]
pub fn p9_rsetattr(tag: u16) -> P9Frame {
    P9Frame::new(P9_RSETATTR, tag, Vec::new())
}

/// Builds a `Txattrwalk` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_txattrwalk(tag: u16, fid: u32, newfid: u32, name: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, newfid);
    push_string(&mut payload, name)?;
    Ok(P9Frame::new(P9_TXATTRWALK, tag, payload))
}

/// Builds an `Rxattrwalk` frame.
#[must_use]
pub fn p9_rxattrwalk(tag: u16, size: u64) -> P9Frame {
    let mut payload = Vec::with_capacity(8);
    push_u64(&mut payload, size);
    P9Frame::new(P9_RXATTRWALK, tag, payload)
}

/// Builds a `Txattrcreate` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_txattrcreate(
    tag: u16,
    fid: u32,
    name: &str,
    attr_size: u64,
    flags: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_string(&mut payload, name)?;
    push_u64(&mut payload, attr_size);
    push_u32(&mut payload, flags);
    Ok(P9Frame::new(P9_TXATTRCREATE, tag, payload))
}

/// Builds an `Rxattrcreate` frame.
#[must_use]
pub fn p9_rxattrcreate(tag: u16) -> P9Frame {
    P9Frame::new(P9_RXATTRCREATE, tag, Vec::new())
}

/// Decodes a `Tgetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tgetattr` or the payload is
/// malformed.
pub fn p9_decode_tgetattr(frame: &P9Frame) -> Result<P9GetAttr, P9Error> {
    expect_message_type(frame, P9_TGETATTR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let request_mask = cursor.read_u64()?;
    cursor.finish()?;
    Ok(P9GetAttr { fid, request_mask })
}

/// Decodes an `Rgetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rgetattr` or the payload is
/// malformed.
pub fn p9_decode_rgetattr(frame: &P9Frame) -> Result<P9Attr, P9Error> {
    expect_message_type(frame, P9_RGETATTR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let attr = cursor.read_attr()?;
    cursor.finish()?;
    Ok(attr)
}

/// Decodes a `Tsetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tsetattr` or the payload is
/// malformed.
pub fn p9_decode_tsetattr(frame: &P9Frame) -> Result<P9SetAttrRequest, P9Error> {
    expect_message_type(frame, P9_TSETATTR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let valid = cursor.read_u32()?;
    let attr = cursor.read_set_attr()?;
    cursor.finish()?;
    Ok(P9SetAttrRequest { fid, valid, attr })
}

/// Decodes an `Rsetattr` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rsetattr` or the payload is not
/// empty.
pub fn p9_decode_rsetattr(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RSETATTR)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Txattrwalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Txattrwalk` or the payload is
/// malformed.
pub fn p9_decode_txattrwalk(frame: &P9Frame) -> Result<P9XattrWalk, P9Error> {
    expect_message_type(frame, P9_TXATTRWALK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let newfid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    cursor.finish()?;
    Ok(P9XattrWalk { fid, newfid, name })
}

/// Decodes an `Rxattrwalk` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rxattrwalk` or the payload is
/// malformed.
pub fn p9_decode_rxattrwalk(frame: &P9Frame) -> Result<u64, P9Error> {
    expect_message_type(frame, P9_RXATTRWALK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let size = cursor.read_u64()?;
    cursor.finish()?;
    Ok(size)
}

/// Decodes a `Txattrcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Txattrcreate` or the payload is
/// malformed.
pub fn p9_decode_txattrcreate(frame: &P9Frame) -> Result<P9XattrCreate, P9Error> {
    expect_message_type(frame, P9_TXATTRCREATE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let attr_size = cursor.read_u64()?;
    let flags = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9XattrCreate {
        fid,
        name,
        attr_size,
        flags,
    })
}

/// Decodes an `Rxattrcreate` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rxattrcreate` or the payload is
/// not empty.
pub fn p9_decode_rxattrcreate(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RXATTRCREATE)?;
    PayloadCursor::new(frame.payload()).finish()
}
