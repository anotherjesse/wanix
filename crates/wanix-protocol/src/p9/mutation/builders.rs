use super::super::codec::{push_qid, push_string, push_u32};
use super::super::{
    P9_RLINK, P9_RMKDIR, P9_RREMOVE, P9_RRENAME, P9_RRENAMEAT, P9_RUNLINKAT, P9_TLINK, P9_TMKDIR,
    P9_TREMOVE, P9_TRENAME, P9_TRENAMEAT, P9_TUNLINKAT, P9Error, P9Frame, P9Qid,
};

const P9_U8_FIELD_LEN: usize = 1;
const P9_U32_FIELD_LEN: usize = 4;
const P9_U64_FIELD_LEN: usize = 8;
const P9_QID_FIELD_LEN: usize = P9_U8_FIELD_LEN + P9_U32_FIELD_LEN + P9_U64_FIELD_LEN;
const P9_TREMOVE_PAYLOAD_LEN: usize = P9_U32_FIELD_LEN;

/// Builds a `Tlink` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tlink(tag: u16, dir_fid: u32, fid: u32, name: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_u32(&mut payload, fid);
    push_string(&mut payload, name)?;
    Ok(P9Frame::new(P9_TLINK, tag, payload))
}

/// Builds an `Rlink` frame.
#[must_use]
pub fn p9_rlink(tag: u16) -> P9Frame {
    P9Frame::new(P9_RLINK, tag, Vec::new())
}

/// Builds a legacy `Trename` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_trename(tag: u16, fid: u32, dir_fid: u32, name: &str) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, fid);
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    Ok(P9Frame::new(P9_TRENAME, tag, payload))
}

/// Builds a legacy `Rrename` frame.
#[must_use]
pub fn p9_rrename(tag: u16) -> P9Frame {
    P9Frame::new(P9_RRENAME, tag, Vec::new())
}

/// Builds a `Tmkdir` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tmkdir(
    tag: u16,
    dir_fid: u32,
    name: &str,
    mode: u32,
    gid: u32,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, mode);
    push_u32(&mut payload, gid);
    Ok(P9Frame::new(P9_TMKDIR, tag, payload))
}

/// Builds an `Rmkdir` frame.
#[must_use]
pub fn p9_rmkdir(tag: u16, qid: P9Qid) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_QID_FIELD_LEN);
    push_qid(&mut payload, qid);
    P9Frame::new(P9_RMKDIR, tag, payload)
}

/// Builds a `Trenameat` frame.
///
/// # Errors
///
/// Returns an error when either name cannot fit in a 9P string field.
pub fn p9_trenameat(
    tag: u16,
    old_dir_fid: u32,
    old_name: &str,
    new_dir_fid: u32,
    new_name: &str,
) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, old_dir_fid);
    push_string(&mut payload, old_name)?;
    push_u32(&mut payload, new_dir_fid);
    push_string(&mut payload, new_name)?;
    Ok(P9Frame::new(P9_TRENAMEAT, tag, payload))
}

/// Builds an `Rrenameat` frame.
#[must_use]
pub fn p9_rrenameat(tag: u16) -> P9Frame {
    P9Frame::new(P9_RRENAMEAT, tag, Vec::new())
}

/// Builds a `Tunlinkat` frame.
///
/// # Errors
///
/// Returns an error when the name cannot fit in a 9P string field.
pub fn p9_tunlinkat(tag: u16, dir_fid: u32, name: &str, flags: u32) -> Result<P9Frame, P9Error> {
    let mut payload = Vec::new();
    push_u32(&mut payload, dir_fid);
    push_string(&mut payload, name)?;
    push_u32(&mut payload, flags);
    Ok(P9Frame::new(P9_TUNLINKAT, tag, payload))
}

/// Builds an `Runlinkat` frame.
#[must_use]
pub fn p9_runlinkat(tag: u16) -> P9Frame {
    P9Frame::new(P9_RUNLINKAT, tag, Vec::new())
}

/// Builds a legacy `Tremove` frame.
#[must_use]
pub fn p9_tremove(tag: u16, fid: u32) -> P9Frame {
    let mut payload = Vec::with_capacity(P9_TREMOVE_PAYLOAD_LEN);
    push_u32(&mut payload, fid);
    P9Frame::new(P9_TREMOVE, tag, payload)
}

/// Builds a legacy `Rremove` frame.
#[must_use]
pub fn p9_rremove(tag: u16) -> P9Frame {
    P9Frame::new(P9_RREMOVE, tag, Vec::new())
}
