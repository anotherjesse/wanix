use super::super::codec::{PayloadCursor, decode_qid_frame, expect_message_type};
use super::super::{
    P9_RLINK, P9_RMKDIR, P9_RREMOVE, P9_RRENAME, P9_RRENAMEAT, P9_RUNLINKAT, P9_TLINK, P9_TMKDIR,
    P9_TREMOVE, P9_TRENAME, P9_TRENAMEAT, P9_TUNLINKAT, P9Error, P9Frame, P9Link, P9Mkdir, P9Qid,
    P9Remove, P9Rename, P9RenameAt, P9UnlinkAt,
};

/// Decodes a `Tlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tlink` or the payload is
/// malformed.
pub fn p9_decode_tlink(frame: &P9Frame) -> Result<P9Link, P9Error> {
    expect_message_type(frame, P9_TLINK)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let link = read_link(&mut cursor)?;
    cursor.finish()?;
    Ok(link)
}

fn read_link(cursor: &mut PayloadCursor<'_>) -> Result<P9Link, P9Error> {
    let dir_fid = cursor.read_u32()?;
    let fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    Ok(P9Link { dir_fid, fid, name })
}

/// Decodes an `Rlink` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rlink` or the payload is not
/// empty.
pub fn p9_decode_rlink(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RLINK)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a legacy `Trename` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Trename` or the payload is
/// malformed.
pub fn p9_decode_trename(frame: &P9Frame) -> Result<P9Rename, P9Error> {
    expect_message_type(frame, P9_TRENAME)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let rename = read_rename(&mut cursor)?;
    cursor.finish()?;
    Ok(rename)
}

fn read_rename(cursor: &mut PayloadCursor<'_>) -> Result<P9Rename, P9Error> {
    let fid = cursor.read_u32()?;
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    Ok(P9Rename { fid, dir_fid, name })
}

/// Decodes a legacy `Rrename` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rrename` or the payload is
/// not empty.
pub fn p9_decode_rrename(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RRENAME)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tmkdir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tmkdir` or the payload is
/// malformed.
pub fn p9_decode_tmkdir(frame: &P9Frame) -> Result<P9Mkdir, P9Error> {
    expect_message_type(frame, P9_TMKDIR)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let mkdir = read_mkdir(&mut cursor)?;
    cursor.finish()?;
    Ok(mkdir)
}

fn read_mkdir(cursor: &mut PayloadCursor<'_>) -> Result<P9Mkdir, P9Error> {
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let mode = cursor.read_u32()?;
    let gid = cursor.read_u32()?;
    Ok(P9Mkdir {
        dir_fid,
        name,
        mode,
        gid,
    })
}

/// Decodes an `Rmkdir` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rmkdir` or the payload is
/// malformed.
pub fn p9_decode_rmkdir(frame: &P9Frame) -> Result<P9Qid, P9Error> {
    decode_qid_frame(frame, P9_RMKDIR)
}

/// Decodes a `Trenameat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Trenameat` or the payload is
/// malformed.
pub fn p9_decode_trenameat(frame: &P9Frame) -> Result<P9RenameAt, P9Error> {
    expect_message_type(frame, P9_TRENAMEAT)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let rename = read_renameat(&mut cursor)?;
    cursor.finish()?;
    Ok(rename)
}

fn read_renameat(cursor: &mut PayloadCursor<'_>) -> Result<P9RenameAt, P9Error> {
    let old_dir_fid = cursor.read_u32()?;
    let old_name = cursor.read_string()?;
    let new_dir_fid = cursor.read_u32()?;
    let new_name = cursor.read_string()?;
    Ok(P9RenameAt {
        old_dir_fid,
        old_name,
        new_dir_fid,
        new_name,
    })
}

/// Decodes an `Rrenameat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rrenameat` or the payload is
/// not empty.
pub fn p9_decode_rrenameat(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RRENAMEAT)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a `Tunlinkat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tunlinkat` or the payload is
/// malformed.
pub fn p9_decode_tunlinkat(frame: &P9Frame) -> Result<P9UnlinkAt, P9Error> {
    expect_message_type(frame, P9_TUNLINKAT)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let unlink = read_unlinkat(&mut cursor)?;
    cursor.finish()?;
    Ok(unlink)
}

fn read_unlinkat(cursor: &mut PayloadCursor<'_>) -> Result<P9UnlinkAt, P9Error> {
    let dir_fid = cursor.read_u32()?;
    let name = cursor.read_string()?;
    let flags = cursor.read_u32()?;
    Ok(P9UnlinkAt {
        dir_fid,
        name,
        flags,
    })
}

/// Decodes an `Runlinkat` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Runlinkat` or the payload is
/// not empty.
pub fn p9_decode_runlinkat(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RUNLINKAT)?;
    PayloadCursor::new(frame.payload()).finish()
}

/// Decodes a legacy `Tremove` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Tremove` or the payload is
/// malformed.
pub fn p9_decode_tremove(frame: &P9Frame) -> Result<P9Remove, P9Error> {
    expect_message_type(frame, P9_TREMOVE)?;
    let mut cursor = PayloadCursor::new(frame.payload());
    let fid = cursor.read_u32()?;
    cursor.finish()?;
    Ok(P9Remove { fid })
}

/// Decodes a legacy `Rremove` frame payload.
///
/// # Errors
///
/// Returns an error when the frame type is not `Rremove` or the payload is not
/// empty.
pub fn p9_decode_rremove(frame: &P9Frame) -> Result<(), P9Error> {
    expect_message_type(frame, P9_RREMOVE)?;
    PayloadCursor::new(frame.payload()).finish()
}
