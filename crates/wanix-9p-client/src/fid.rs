//! Fid and tag allocation with free-lists, plus an RAII scratch-fid guard.
//!
//! A long-lived mount issues a great many walks, opens, and reads. Naively
//! incrementing a counter would eventually wrap `u32` fids or `u16` tags and
//! collide with live state on the server. [`FidPool`] and [`TagPool`] therefore
//! recycle released numbers through a free-list.
//!
//! [`ScratchFid`] is the leak guard required by the crate contract: any fid
//! produced by a walk must be clunked on every exit path, including a panic
//! unwinding through the caller. The guard holds a clone of the shared
//! connection and best-effort clunks its fid on [`Drop`].

use std::sync::{Arc, Mutex};

use crate::conn::P9Conn;

/// The 9P `NOFID` sentinel, never handed out by the pool.
const NOFID: u32 = 0xffff_ffff;

/// The 9P `NOTAG` sentinel, reserved for version negotiation.
const NOTAG: u16 = 0xffff;

/// Allocator for 9P fids backed by a free-list.
///
/// Fids are drawn from an ascending counter until one is released, after which
/// released fids are reused in preference to advancing the counter. The `NOFID`
/// sentinel is never produced.
#[derive(Debug, Default)]
pub struct FidPool {
    next: u32,
    free: Vec<u32>,
}

impl FidPool {
    /// Creates an empty fid pool that begins allocating at fid `1`.
    ///
    /// Fid `0` is conventionally the attach (root) fid and is reserved by the
    /// connection, so the pool starts above it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 1,
            free: Vec::new(),
        }
    }

    /// Returns the next available fid, reusing a freed fid when possible.
    ///
    /// Returns `None` only when every non-sentinel fid is simultaneously in use,
    /// which a single serial client cannot reach in practice.
    pub fn allocate(&mut self) -> Option<u32> {
        if let Some(fid) = self.free.pop() {
            return Some(fid);
        }
        if self.next == NOFID {
            return None;
        }
        let fid = self.next;
        self.next += 1;
        Some(fid)
    }

    /// Returns `fid` to the free-list for reuse.
    pub fn release(&mut self, fid: u32) {
        if fid != NOFID {
            self.free.push(fid);
        }
    }
}

/// Allocator for 9P tags backed by a free-list.
///
/// Because the client keeps a single request outstanding at a time, a tiny
/// number of tags is ever live, but the free-list still guarantees a long-lived
/// session never wraps `u16` or reuses the `NOTAG` sentinel.
#[derive(Debug, Default)]
pub struct TagPool {
    next: u16,
    free: Vec<u16>,
}

impl TagPool {
    /// Creates an empty tag pool that begins allocating at tag `0`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 0,
            free: Vec::new(),
        }
    }

    /// Returns the next available tag, reusing a freed tag when possible.
    ///
    /// Returns `None` only when every non-sentinel tag is simultaneously in use.
    pub fn allocate(&mut self) -> Option<u16> {
        if let Some(tag) = self.free.pop() {
            return Some(tag);
        }
        if self.next == NOTAG {
            return None;
        }
        let tag = self.next;
        self.next += 1;
        Some(tag)
    }

    /// Returns `tag` to the free-list for reuse.
    pub fn release(&mut self, tag: u16) {
        if tag != NOTAG {
            self.free.push(tag);
        }
    }
}

/// RAII guard owning a walked fid that clunks it on drop.
///
/// The guard holds a clone of the shared connection so its [`Drop`] can lock the
/// mutex and send a best-effort `Tclunk`, returning the fid to the pool. This
/// makes every error path between a walk and the operation that consumes the fid
/// leak-free, including a panic unwinding through the caller.
pub struct ScratchFid {
    conn: Arc<Mutex<P9Conn>>,
    fid: u32,
    released: bool,
}

impl ScratchFid {
    /// Wraps `fid` walked on `conn` in a clunk-on-drop guard.
    #[must_use]
    pub fn new(conn: Arc<Mutex<P9Conn>>, fid: u32) -> Self {
        Self {
            conn,
            fid,
            released: false,
        }
    }

    /// Returns the guarded fid number.
    #[must_use]
    pub fn fid(&self) -> u32 {
        self.fid
    }

    /// Surrenders the fid to the caller without clunking it on drop.
    ///
    /// Used when ownership of the fid transfers to a longer-lived handle (an open
    /// [`crate::file::RemoteFile`]), which then owns the clunk responsibility.
    #[must_use]
    pub fn into_fid(mut self) -> u32 {
        self.released = true;
        self.fid
    }
}

impl Drop for ScratchFid {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        // Best-effort: a clunk failure on a dying scratch fid cannot be acted on,
        // and a poisoned mutex means the connection is already unusable.
        if let Ok(mut conn) = self.conn.lock() {
            conn.clunk_fid(self.fid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fid_pool_skips_reserved_root_fid() {
        let mut pool = FidPool::new();
        assert_eq!(pool.allocate(), Some(1));
        assert_eq!(pool.allocate(), Some(2));
    }

    #[test]
    fn fid_pool_reuses_released_fids_before_advancing() {
        let mut pool = FidPool::new();
        let a = pool.allocate().unwrap();
        let b = pool.allocate().unwrap();
        pool.release(a);
        assert_eq!(pool.allocate(), Some(a));
        assert_ne!(b, a);
    }

    #[test]
    fn tag_pool_reuses_released_tags() {
        let mut pool = TagPool::new();
        let a = pool.allocate().unwrap();
        pool.release(a);
        assert_eq!(pool.allocate(), Some(a));
    }

    #[test]
    fn pools_never_emit_sentinels() {
        let mut fids = FidPool {
            next: NOFID,
            free: Vec::new(),
        };
        assert_eq!(fids.allocate(), None);
        let mut tags = TagPool {
            next: NOTAG,
            free: Vec::new(),
        };
        assert_eq!(tags.allocate(), None);
        fids.release(NOFID);
        tags.release(NOTAG);
        assert!(fids.free.is_empty());
        assert!(tags.free.is_empty());
    }
}
