//! [`LineBuffer`]: the blocking byte buffer behind one stream subscription.
//!
//! A direct copy of the `wanix-plumb` subscription-buffer discipline: a push
//! always notifies, a read blocks on a condvar with a periodic re-check (to
//! bound a missed wakeup), and `Ok(0)` is returned only once the buffer is
//! permanently closed — which the AppFS adapter never does for a live stream
//! file, making `stream` reads honestly never-EOF while the host owns them.
//!
//! The type is exported so hosts can reuse the same bounded drop-oldest
//! discipline for other guest-fed byte captures (e.g. a resident guest's
//! stderr) instead of growing an unbounded buffer or copying the pattern.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use wanix_fs::{FsError, FsResult};

/// Worst-case re-check interval for a blocked read. A push always notifies, so
/// this only bounds a missed wakeup; it is never an end-of-stream signal.
const READ_RECHECK_INTERVAL: Duration = Duration::from_millis(50);

/// Hard ceiling on the bytes one stream subscription buffers before its
/// reader has drained them. A slow or absent reader must not let guest
/// publishes grow a buffer without bound: once the ceiling is reached, the
/// oldest buffered bytes are dropped to make room — the explicit lossy
/// slow-reader policy `docs/appfs.md` requires. Equals
/// [`crate::MAX_LINE_LEN`], so one maximal publish at most fills one buffer.
pub(crate) const MAX_BUFFERED_BYTES: usize = crate::MAX_LINE_LEN;

#[derive(Default)]
struct Inner {
    data: VecDeque<u8>,
    closed: bool,
}

/// A bounded, lossy, blocking byte buffer: the adapter fans publishes into
/// it and a stream subscriber drains it; hosts may also use it directly for
/// bounded guest-fed captures.
#[derive(Default)]
pub struct LineBuffer {
    inner: Mutex<Inner>,
    signal: Condvar,
}

impl LineBuffer {
    fn lock(&self) -> FsResult<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| FsError::Other("app stream buffer lock poisoned".to_owned()))
    }

    /// Appends published bytes and wakes a blocked reader.
    ///
    /// Enforces [`MAX_BUFFERED_BYTES`]: if appending `bytes` would exceed the
    /// per-subscription ceiling, the oldest buffered bytes are dropped to make
    /// room first, so a slow reader never grows the buffer without bound and
    /// always sees the most recent backlog. A single push larger than the
    /// whole ceiling keeps only its trailing `MAX_BUFFERED_BYTES`.
    pub fn push(&self, bytes: &[u8]) {
        if let Ok(mut inner) = self.inner.lock() {
            // Drop the oldest bytes until the new data fits under the ceiling.
            let incoming = bytes.len().min(MAX_BUFFERED_BYTES);
            while inner.data.len() + incoming > MAX_BUFFERED_BYTES {
                let overflow = inner.data.len() + incoming - MAX_BUFFERED_BYTES;
                let drop = overflow.min(inner.data.len());
                if drop == 0 {
                    break;
                }
                inner.data.drain(..drop);
            }
            // If the push itself is larger than the ceiling, keep only its tail.
            let start = bytes.len().saturating_sub(MAX_BUFFERED_BYTES);
            inner.data.extend(bytes[start..].iter().copied());
        }
        self.signal.notify_all();
    }

    /// Marks the buffer permanently closed so a blocked reader observes EOF.
    ///
    /// Stream files are never-EOF while the guest lives; this is the
    /// host-side teardown ([`crate::AppStreamCloser`]) releasing blocked
    /// readers once the guest app has exited.
    pub fn close(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.closed = true;
        }
        self.signal.notify_all();
    }

    /// Drains buffered bytes, blocking until data arrives or the buffer closes.
    /// Returns `Ok(0)` only at end-of-stream (no data and closed).
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Other`] when the buffer lock or wait is poisoned.
    pub fn read(&self, buf: &mut [u8]) -> FsResult<usize> {
        let mut inner = self.lock()?;
        loop {
            if !inner.data.is_empty() {
                let len = inner.data.len().min(buf.len());
                for slot in buf.iter_mut().take(len) {
                    if let Some(byte) = inner.data.pop_front() {
                        *slot = byte;
                    }
                }
                return Ok(len);
            }
            if inner.closed {
                return Ok(0);
            }
            let (next, _timed_out) = self
                .signal
                .wait_timeout(inner, READ_RECHECK_INTERVAL)
                .map_err(|_| FsError::Other("app stream buffer wait poisoned".to_owned()))?;
            inner = next;
        }
    }

    /// Whether a read would return without blocking (data buffered or closed).
    ///
    /// # Errors
    ///
    /// Returns [`FsError::Other`] when the buffer lock is poisoned.
    pub fn read_ready(&self) -> FsResult<bool> {
        let inner = self.lock()?;
        Ok(!inner.data.is_empty() || inner.closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_drains_pushed_bytes() {
        let buffer = LineBuffer::default();
        buffer.push(b"hello");
        let mut buf = [0_u8; 8];
        assert_eq!(buffer.read(&mut buf).unwrap(), 5);
        assert_eq!(&buf[..5], b"hello");
    }

    #[test]
    fn read_reports_eof_only_after_close() {
        let buffer = LineBuffer::default();
        assert!(!buffer.read_ready().unwrap());
        buffer.close();
        let mut buf = [0_u8; 8];
        assert_eq!(buffer.read(&mut buf).unwrap(), 0);
        assert!(buffer.read_ready().unwrap());
    }

    #[test]
    fn push_caps_buffer_and_keeps_newest() {
        let buffer = LineBuffer::default();
        let filler = vec![b'o'; MAX_BUFFERED_BYTES];
        buffer.push(&filler);
        buffer.push(b"NEWEST");
        let buffered = buffer.inner.lock().unwrap().data.len();
        assert!(buffered <= MAX_BUFFERED_BYTES);
        let mut buf = vec![0_u8; MAX_BUFFERED_BYTES];
        let mut drained = Vec::new();
        while buffer.read_ready().unwrap() {
            let n = buffer.read(&mut buf).unwrap();
            drained.extend_from_slice(&buf[..n]);
        }
        assert!(
            drained.ends_with(b"NEWEST"),
            "the most recently pushed bytes must survive eviction"
        );
    }
}
