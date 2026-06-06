//! Path-addressed 9P operations for [`RemoteFs`] that target a parent directory.
//!
//! 9P creation and removal address a child by `(dir_fid, name)`, and rename
//! addresses both sides the same way. Each operation here walks to the relevant
//! parent fid, derives the basename, and issues the matching `Tmkdir`,
//! `Tunlinkat`, or `Trenameat`. Directory listing opens the walked fid and pages
//! it through the bounded `Treaddir` loop.

use wanix_fs::{DirEntry, FsError, NormalizedPath};
use wanix_protocol::{
    p9_decode_rlopen, p9_decode_rmkdir, p9_decode_rreadlink, p9_decode_rrenameat,
    p9_decode_runlinkat, p9_tlopen, p9_tmkdir, p9_treadlink, p9_trenameat, p9_tunlinkat,
};

use crate::error::{ClientError, ClientResult};
use crate::readdir::read_all_entries;
use crate::remote::RemoteFs;
use crate::walk::{lock, walk_to};

/// Linux `unlinkat` flag selecting directory removal (`AT_REMOVEDIR`).
const AT_REMOVEDIR: u32 = 0x200;

/// Mode bits requested when creating a directory (`0o755`).
const DIR_MODE: u32 = 0o755;

/// Default group id for created nodes; the server owns real ownership policy.
const DEFAULT_GID: u32 = 0;

/// Opens the directory at `path` and pages every entry into a listing.
pub(crate) fn read_dir(fs: &RemoteFs, path: &NormalizedPath) -> ClientResult<Vec<DirEntry>> {
    let guard = walk_to(&fs.conn(), path)?;
    let fid = guard.fid();
    let conn = fs.conn();
    let mut held = lock(&conn)?;
    let reply = held.rpc(|tag| Ok(p9_tlopen(tag, fid, 0)))?;
    p9_decode_rlopen(&reply)?;
    read_all_entries(&mut held, fid)
}

/// Creates one directory named by the final component of `path`.
pub(crate) fn create_dir(fs: &RemoteFs, path: &NormalizedPath) -> ClientResult<()> {
    let (parent, name) = split_parent(path)?;
    let guard = walk_to(&fs.conn(), &parent)?;
    let dir_fid = guard.fid();
    let conn = fs.conn();
    let mut held = lock(&conn)?;
    let reply = held.rpc(|tag| p9_tmkdir(tag, dir_fid, &name, DIR_MODE, DEFAULT_GID))?;
    p9_decode_rmkdir(&reply)?;
    Ok(())
}

/// Removes the child named by the final component of `path`.
///
/// `directory` selects `AT_REMOVEDIR` so the server removes an (empty) directory
/// rather than a regular file.
pub(crate) fn remove(fs: &RemoteFs, path: &NormalizedPath, directory: bool) -> ClientResult<()> {
    let (parent, name) = split_parent(path)?;
    let guard = walk_to(&fs.conn(), &parent)?;
    let dir_fid = guard.fid();
    let flags = if directory { AT_REMOVEDIR } else { 0 };
    let conn = fs.conn();
    let mut held = lock(&conn)?;
    let reply = held.rpc(|tag| p9_tunlinkat(tag, dir_fid, &name, flags))?;
    p9_decode_runlinkat(&reply)?;
    Ok(())
}

/// Renames `old_path` to `new_path` via `Trenameat` on both parent fids.
pub(crate) fn rename(
    fs: &RemoteFs,
    old_path: &NormalizedPath,
    new_path: &NormalizedPath,
) -> ClientResult<()> {
    let (old_parent, old_name) = split_parent(old_path)?;
    let (new_parent, new_name) = split_parent(new_path)?;
    let old_guard = walk_to(&fs.conn(), &old_parent)?;
    let new_guard = walk_to(&fs.conn(), &new_parent)?;
    let old_dir_fid = old_guard.fid();
    let new_dir_fid = new_guard.fid();
    let conn = fs.conn();
    let mut held = lock(&conn)?;
    let reply =
        held.rpc(|tag| p9_trenameat(tag, old_dir_fid, &old_name, new_dir_fid, &new_name))?;
    p9_decode_rrenameat(&reply)?;
    Ok(())
}

/// Reads the raw target bytes of the symbolic link at `path`.
pub(crate) fn read_link(fs: &RemoteFs, path: &NormalizedPath) -> ClientResult<Vec<u8>> {
    let guard = walk_to(&fs.conn(), path)?;
    let fid = guard.fid();
    let conn = fs.conn();
    let mut held = lock(&conn)?;
    let reply = held.rpc(|tag| Ok(p9_treadlink(tag, fid)))?;
    let target = p9_decode_rreadlink(&reply)?;
    Ok(target.into_bytes())
}

/// Splits a non-root path into its parent path and final component name.
///
/// The root path has no parent, so a creation or removal targeting it is an
/// invalid request rather than a server round trip.
fn split_parent(path: &NormalizedPath) -> ClientResult<(NormalizedPath, String)> {
    let parent = path.parent().ok_or_else(|| {
        ClientError::Request(FsError::Other(
            "cannot mutate the filesystem root".to_owned(),
        ))
    })?;
    Ok((parent, path.file_name().to_owned()))
}
