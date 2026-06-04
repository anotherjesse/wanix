use super::codec::{
    PayloadCursor, decode_data_frame, expect_message_type, push_counted_data, push_u32, push_u64,
};
use super::{P9_RREAD, P9_RWRITE, P9_TREAD, P9_TWRITE, P9Error, P9Frame, P9Read, P9Write};

/// Builds a `Tread` frame.
#[must_use]
pub fn p9_tread(tag: u16, fid: u32, offset: u64, count: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(16);
    push_u32(&mut payload, fid);
    push_u64(&mut payload, offset);
    push_u32(&mut payload, count);
    P9Frame::new(P9_TREAD, tag, payload)
}

/// Builds an `Rread` frame.
///
/// # Errors
///
/// Returns an error when the data cannot fit in a 9P u32 count field.
pub fn p9_rread(tag: u16, data: &[u8]) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::with_capacity(4 + data.len());
    push_counted_data(&mut payload, data)?;
    Ok(P9Frame::new(P9_RREAD, tag, payload))
}

/// Builds a `Twrite` frame.
///
/// # Errors
///
/// Returns an error when the data cannot fit in a 9P u32 count field.
pub fn p9_twrite(tag: u16, fid: u32, offset: u64, data: &[u8]) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::with_capacity(16 + data.len());
    push_u32(&mut payload, fid);
    push_u64(&mut payload, offset);
    push_counted_data(&mut payload, data)?;
    Ok(P9Frame::new(P9_TWRITE, tag, payload))
}

/// Builds an `Rwrite` frame.
#[must_use]
pub fn p9_rwrite(tag: u16, count: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(4);
    push_u32(&mut payload, count);
    P9Frame::new(P9_RWRITE, tag, payload)
}

/// Decodes a `Tread` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tread` or the payload is
/// malformed.
pub fn p9_decode_tread(frame: &P9Frame) -> Result<P9Read, P9Error> {
    expect_message_type(frame, P9_TREAD)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let request = read_read_request(&mut cursor)?;
    cursor.finish()?;
    Ok(request)
}

fn read_read_request(cursor: &mut PayloadCursor<'_>) -> Result<P9Read, P9Error> {
    let fid = cursor.read_u32()?;
    let offset = cursor.read_u64()?;
    let count = cursor.read_u32()?;
    Ok(P9Read { fid, offset, count })
}

/// Decodes an `Rread` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rread` or the payload is
/// malformed.
pub fn p9_decode_rread(frame: &P9Frame) -> Result<Vec<u8>, P9Error> {
    decode_data_frame(frame, P9_RREAD)
}

/// Decodes a `Twrite` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Twrite` or the payload is
/// malformed.
pub fn p9_decode_twrite(frame: &P9Frame) -> Result<P9Write, P9Error> {
    expect_message_type(frame, P9_TWRITE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let request = read_write_request(&mut cursor)?;
    cursor.finish()?;
    Ok(request)
}

fn read_write_request(cursor: &mut PayloadCursor<'_>) -> Result<P9Write, P9Error> {
    let fid = cursor.read_u32()?;
    let offset = cursor.read_u64()?;
    let data = cursor.read_counted_data()?;
    Ok(P9Write { fid, offset, data })
}

/// Decodes an `Rwrite` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rwrite` or the payload is
/// malformed.
pub fn p9_decode_rwrite(frame: &P9Frame) -> Result<u32, P9Error> {
    expect_message_type(frame, P9_RWRITE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let count = cursor.read_u32()?;
    cursor.finish()?;
    Ok(count)
}
