use super::super::codec::{push_qid, push_string, push_u32};
use super::super::{
    P9_RLCREATE, P9_RLOPEN, P9_RMKNOD, P9_RREADLINK, P9_RSYMLINK, P9_TLCREATE, P9_TLOPEN,
    P9_TMKNOD, P9_TREADLINK, P9_TSYMLINK, P9Error, P9Frame, P9Qid,
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
