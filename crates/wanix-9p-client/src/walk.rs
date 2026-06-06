//! Path walking: clone-walk from the attach root to a guarded scratch fid.
//!
//! Every [`wanix_fs::FileSystem`] method names a path, but 9P addresses files by
//! fid. [`walk_to`] clone-walks from the immutable root fid to a fresh fid bound
//! to `path`, chunking components to at most [`MAXWELEM`] names per `Twalk` as
//! the protocol requires. The result is a [`ScratchFid`] that clunks itself on
//! every exit path.
//!
//! When the server speaks the Google.2 extension, [`walkgetattr_to`] folds the
//! final `Tgetattr` into the walk, saving a round trip on metadata-bearing
//! operations.

use std::sync::{Arc, Mutex};

use wanix_fs::NormalizedPath;
use wanix_protocol::{
    P9Attr, P9AttrBody, P9Qid, p9_decode_rwalk, p9_decode_rwalkgetattr, p9_twalk, p9_twalkgetattr,
};

use crate::conn::{P9Conn, ROOT_FID};
use crate::error::{ClientError, ClientResult};
use crate::fid::ScratchFid;

/// Maximum path components a single `Twalk`/`Twalkgetattr` may carry.
pub const MAXWELEM: usize = 16;

/// Walks from the attach root to `path`, returning a clunk-on-drop fid guard.
///
/// The walk clones the root fid into a freshly allocated fid and walks the path
/// components in chunks of at most [`MAXWELEM`]. A partial walk (the server
/// returns fewer QIDs than names) means an intermediate component was missing,
/// reported as [`wanix_fs::FsError::NotFound`] via [`ClientError::Remote`].
///
/// # Errors
///
/// Returns [`ClientError`] when path components are invalid, fid allocation
/// fails, or any `Twalk` exchange fails.
pub fn walk_to(conn: &Arc<Mutex<P9Conn>>, path: &NormalizedPath) -> ClientResult<ScratchFid> {
    let components = path_components(path)?;
    let newfid = lock(conn)?.allocate_fid()?;
    let guard = ScratchFid::new(Arc::clone(conn), newfid);

    if components.is_empty() {
        clone_root(conn, newfid)?;
        return Ok(guard);
    }

    let mut first = true;
    for chunk in components.chunks(MAXWELEM) {
        let source = if first { ROOT_FID } else { newfid };
        let qids = walk_chunk(conn, source, newfid, chunk)?;
        if qids.len() != chunk.len() {
            return Err(ClientError::Remote {
                errno: crate::error::ENOENT,
            });
        }
        first = false;
    }
    Ok(guard)
}

/// Walks to `path` and returns both a fid guard and the final file's attributes.
///
/// Uses `Twalkgetattr` to obtain the attributes in the same exchange. The caller
/// must have confirmed [`P9Conn::supports_walkgetattr`]; otherwise the server
/// rejects the request.
///
/// # Errors
///
/// Returns [`ClientError`] when the walk fails or the server does not support
/// the Google.2 `Twalkgetattr` extension.
pub fn walkgetattr_to(
    conn: &Arc<Mutex<P9Conn>>,
    path: &NormalizedPath,
) -> ClientResult<(ScratchFid, P9Attr)> {
    let components = path_components(path)?;
    let newfid = lock(conn)?.allocate_fid()?;
    let guard = ScratchFid::new(Arc::clone(conn), newfid);

    let names: Vec<&str> = components.iter().map(String::as_str).collect();
    let response = {
        let mut held = lock(conn)?;
        let reply = held.rpc(|tag| p9_twalkgetattr(tag, ROOT_FID, newfid, &names))?;
        p9_decode_rwalkgetattr(&reply)?
    };
    if response.qids.len() != names.len() {
        return Err(ClientError::Remote {
            errno: crate::error::ENOENT,
        });
    }
    let qid = response.qids.last().copied().unwrap_or(ROOT_QID);
    Ok((guard, attr_from_body(qid, response.valid, response.attr)))
}

/// QID stand-in for the root when a zero-component walkgetattr returns no QIDs.
const ROOT_QID: P9Qid = P9Qid {
    qid_type: 0,
    version: 0,
    path: 0,
};

fn clone_root(conn: &Arc<Mutex<P9Conn>>, newfid: u32) -> ClientResult<()> {
    let mut held = lock(conn)?;
    let reply = held.rpc(|tag| p9_twalk(tag, ROOT_FID, newfid, &[]))?;
    p9_decode_rwalk(&reply)?;
    Ok(())
}

fn walk_chunk(
    conn: &Arc<Mutex<P9Conn>>,
    source: u32,
    newfid: u32,
    chunk: &[String],
) -> ClientResult<Vec<P9Qid>> {
    let names: Vec<&str> = chunk.iter().map(String::as_str).collect();
    let mut held = lock(conn)?;
    let reply = held.rpc(|tag| p9_twalk(tag, source, newfid, &names))?;
    Ok(p9_decode_rwalk(&reply)?)
}

/// Reassembles a full [`P9Attr`] from a walkgetattr body, qid, and valid mask.
fn attr_from_body(qid: P9Qid, valid: u64, body: P9AttrBody) -> P9Attr {
    P9Attr {
        valid,
        qid,
        mode: body.mode,
        uid: body.uid,
        gid: body.gid,
        nlink: body.nlink,
        rdev: body.rdev,
        size: body.size,
        block_size: body.block_size,
        blocks: body.blocks,
        atime_seconds: body.atime_seconds,
        atime_nanoseconds: body.atime_nanoseconds,
        mtime_seconds: body.mtime_seconds,
        mtime_nanoseconds: body.mtime_nanoseconds,
        ctime_seconds: body.ctime_seconds,
        ctime_nanoseconds: body.ctime_nanoseconds,
        btime_seconds: body.btime_seconds,
        btime_nanoseconds: body.btime_nanoseconds,
        generation: body.generation,
        data_version: body.data_version,
    }
}

/// Splits a normalized path into its non-root components.
fn path_components(path: &NormalizedPath) -> ClientResult<Vec<String>> {
    if path.as_str() == "." {
        return Ok(Vec::new());
    }
    Ok(path.as_str().split('/').map(str::to_owned).collect())
}

/// Locks the shared connection, mapping poison to a client error.
pub fn lock(conn: &Arc<Mutex<P9Conn>>) -> ClientResult<std::sync::MutexGuard<'_, P9Conn>> {
    conn.lock()
        .map_err(|_| ClientError::Poisoned("9P connection mutex poisoned".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_path_has_no_components() {
        let root = NormalizedPath::new(".").unwrap();
        assert!(path_components(&root).unwrap().is_empty());
    }

    #[test]
    fn nested_path_splits_into_components() {
        let path = NormalizedPath::new("a/b/c").unwrap();
        assert_eq!(path_components(&path).unwrap(), vec!["a", "b", "c"]);
    }
}
