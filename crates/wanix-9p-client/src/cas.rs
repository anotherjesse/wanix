//! Client side of the control/data split: learn a file's [`ContentHash`] over
//! 9P so a CAS-aware caller can offload bulk reads to the blob plane.
//!
//! The hash rides the wire as the `cas.hash` extended attribute — a genuine
//! synthetic file the server serves as 64 lowercase-hex bytes (never as bytes
//! appended to a fixed-shape `Rgetattr`). [`content_hash_for`] walks to the
//! path, issues `Txattrwalk("cas.hash")` onto a fresh guarded fid, and reads the
//! value with one `Tread`:
//!
//! - `Rxattrwalk` + a 64-hex `Tread` → `Some(ContentHash)`: the caller may skip
//!   the `Tread` loop and fetch the BLAKE3-verified blob peer-to-peer.
//! - `ENODATA` → `None`: the file offers no offloadable hash (small, mid-write,
//!   or a filesystem with no blob backing), so the caller reads it the ordinary
//!   way. This is *not* an error — it is the documented fall-back path.
//!
//! This module is iroh-free and transport-free: it only surfaces the hash. The
//! actual peer-to-peer blob fetch lives in `wanix-mesh` (the data plane), which
//! consumes the hash this returns.

use std::sync::{Arc, Mutex};

use wanix_fs::{ContentHash, FsError, NormalizedPath};
use wanix_protocol::{p9_decode_rread, p9_decode_rxattrwalk, p9_tread, p9_txattrwalk};

use crate::conn::P9Conn;
use crate::error::{ClientError, ClientResult, ENODATA};
use crate::fid::ScratchFid;
use crate::walk::{lock, walk_to};

/// Extended-attribute name carrying the BLAKE3 content hash.
pub(crate) const CAS_HASH_XATTR: &str = "cas.hash";

/// Number of lowercase-hex characters in a serialized [`ContentHash`].
const CAS_HASH_HEX_LEN: usize = 64;

/// Walks to `path` and fetches its `cas.hash` xattr, returning the file's
/// [`ContentHash`] when the server offers one.
///
/// Returns `Ok(None)` when the server replies `ENODATA` (the file has no
/// offloadable hash), so the caller transparently falls back to a `Tread` loop.
///
/// # Errors
///
/// Returns [`ClientError`] when the walk fails, the xattr exchange fails for any
/// reason other than `ENODATA`, or the returned value is not a 64-hex hash.
pub(crate) fn content_hash_for(
    conn: &Arc<Mutex<P9Conn>>,
    path: &NormalizedPath,
) -> ClientResult<Option<ContentHash>> {
    let guard = walk_to(conn, path)?;
    let target_fid = guard.fid();

    // Allocate the xattr value fid up front so a guard reclaims it on any exit.
    let value_fid = lock(conn)?.allocate_fid()?;
    let value_guard = ScratchFid::new(Arc::clone(conn), value_fid);

    // The `Txattrwalk` exchange must not hold the connection lock across the
    // ENODATA bookkeeping, so the rpc result is captured and the lock released
    // before any further locking.
    let walk_result = {
        let mut held = lock(conn)?;
        held.rpc(|tag| p9_txattrwalk(tag, target_fid, value_fid, CAS_HASH_XATTR))
    };
    let size = match walk_result {
        Ok(reply) => p9_decode_rxattrwalk(&reply)?,
        // ENODATA is the contract's "no hash" signal, not a failure: the server
        // never created this newfid, so it must not be clunked on the wire.
        // Defuse the guard and return the number to the local pool.
        Err(ClientError::Remote { errno }) if errno == ENODATA => {
            let fid = value_guard.into_fid();
            lock(conn)?.free_fid(fid);
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if size as usize != CAS_HASH_HEX_LEN {
        // A well-behaved server advertises exactly 64 hex bytes; any other size
        // is a protocol violation by the peer, surfaced as a request error.
        return Err(ClientError::Request(FsError::Other(format!(
            "cas.hash xattr size {size}, expected {CAS_HASH_HEX_LEN}"
        ))));
    }

    let hex = read_value(conn, value_fid, CAS_HASH_HEX_LEN)?;
    drop(value_guard);
    let hex = String::from_utf8(hex).map_err(|_| {
        ClientError::Request(FsError::Other("cas.hash xattr is not utf-8".to_owned()))
    })?;
    let hash = ContentHash::from_hex(&hex).map_err(ClientError::Request)?;
    Ok(Some(hash))
}

/// Reads exactly `len` bytes from the open xattr `fid` with one `Tread`.
fn read_value(conn: &Arc<Mutex<P9Conn>>, fid: u32, len: usize) -> ClientResult<Vec<u8>> {
    let mut held = lock(conn)?;
    let reply = held.rpc(|tag| Ok(p9_tread(tag, fid, 0, len as u32)))?;
    Ok(p9_decode_rread(&reply)?)
}
