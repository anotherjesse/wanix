//! [`LineBuffer`]: a blocking, in-memory byte buffer for a `#plumb` subscription.
//!
//! A subscription's `recv` stream needs the same blocking-read-until-data-or-EOF
//! discipline the pipe and agent event streams use: a push always notifies, a
//! read blocks on a condvar with a periodic re-check (to bound a missed wakeup),
//! and `Ok(0)` is returned only once the buffer is permanently closed. This is
//! shared by the in-process [`crate::LocalPlumbPort`] and by `wanix-mesh`'s
//! gossip port so both deliver received envelope bytes identically.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use wanix_fs::{FsError, FsResult};

/// Worst-case re-check interval for a blocked read. A push always notifies, so
/// this only bounds a missed wakeup; it is never an end-of-stream signal.
const READ_RECHECK_INTERVAL: Duration = Duration::from_millis(50);

/// Hard ceiling on the bytes a single subscription buffers before a reader has
/// drained them. Delivery is best-effort epidemic pub/sub, so a `recv`
/// subscriber that never reads (or reads too slowly) must not let a peer flood
/// it into unbounded growth: a remote node that knows a topic name could push an
/// arbitrary number of (individually bounded) gossip messages into an idle
/// subscriber's buffer. Once this ceiling is reached, the oldest buffered bytes
/// are dropped to make room — the same lossy semantics gossip already has for a
/// lagged subscriber. Sized to hold a useful backlog of full
/// [`crate::MAX_ENVELOPE_LEN`]-bounded envelopes (16 messages) while keeping a
/// hard per-subscriber cap.
const MAX_BUFFERED_BYTES: usize = 16 * crate::MAX_ENVELOPE_LEN;

#[derive(Default)]
struct Inner {
    data: VecDeque<u8>,
    closed: bool,
}

/// A blocking byte buffer a port pushes received envelope lines into and a
/// `recv` reader drains.
#[derive(Default)]
pub(crate) struct LineBuffer {
    inner: Mutex<Inner>,
    signal: Condvar,
}

impl LineBuffer {
    fn lock(&self) -> FsResult<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| FsError::Other("plumb line buffer lock poisoned".to_owned()))
    }

    /// Appends received bytes and wakes a blocked reader.
    ///
    /// Enforces [`MAX_BUFFERED_BYTES`]: if appending `bytes` would exceed the
    /// per-subscription ceiling, the oldest buffered bytes are dropped to make
    /// room first, so a slow or idle reader being flooded by a peer never grows
    /// without bound. This is intentionally lossy (best-effort delivery already
    /// drops messages for a lagged subscriber). A single push larger than the
    /// whole ceiling — which cannot happen for a real envelope, capped at
    /// [`crate::MAX_ENVELOPE_LEN`] — keeps only its trailing `MAX_BUFFERED_BYTES`.
    pub(crate) fn push(&self, bytes: &[u8]) {
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
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn close(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.closed = true;
        }
        self.signal.notify_all();
    }

    /// Drains buffered bytes, blocking until data arrives or the buffer closes.
    /// Returns `Ok(0)` only at end-of-stream (no data and closed).
    pub(crate) fn read(&self, buf: &mut [u8]) -> FsResult<usize> {
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
                .map_err(|_| FsError::Other("plumb line buffer wait poisoned".to_owned()))?;
            inner = next;
        }
    }

    /// Whether a read would return without blocking (data buffered or closed).
    pub(crate) fn read_ready(&self) -> FsResult<bool> {
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
    fn read_reports_eof_after_close() {
        let buffer = LineBuffer::default();
        buffer.close();
        let mut buf = [0_u8; 8];
        assert_eq!(buffer.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn read_ready_tracks_data_and_close() {
        let buffer = LineBuffer::default();
        assert!(!buffer.read_ready().unwrap());
        buffer.push(b"x");
        assert!(buffer.read_ready().unwrap());
    }

    #[test]
    fn push_caps_total_buffered_bytes_against_a_flood() {
        // A peer floods an idle subscriber with far more than the ceiling. The
        // buffer must never exceed MAX_BUFFERED_BYTES: oldest bytes are dropped.
        let buffer = LineBuffer::default();
        let chunk = vec![b'a'; crate::MAX_ENVELOPE_LEN];
        // Push many full-sized envelopes without ever reading.
        for _ in 0..1000 {
            buffer.push(&chunk);
        }
        let buffered = buffer.inner.lock().unwrap().data.len();
        assert!(
            buffered <= MAX_BUFFERED_BYTES,
            "buffer grew to {buffered} bytes, above the {MAX_BUFFERED_BYTES}-byte ceiling"
        );
    }

    #[test]
    fn push_drops_oldest_so_newest_survives() {
        // When the ceiling is reached, the newest bytes are retained and the
        // oldest are evicted: a draining reader sees recent data, not stale.
        let buffer = LineBuffer::default();
        let filler = vec![b'o'; MAX_BUFFERED_BYTES];
        buffer.push(&filler);
        buffer.push(b"NEWEST");
        let mut buf = vec![0_u8; MAX_BUFFERED_BYTES];
        let n = buffer.read(&mut buf).unwrap();
        assert!(
            buf[..n].ends_with(b"NEWEST"),
            "the most recently pushed bytes must survive eviction"
        );
        assert!(n <= MAX_BUFFERED_BYTES);
    }

    #[test]
    fn push_larger_than_ceiling_keeps_only_its_tail() {
        // A single push bigger than the whole buffer keeps only its trailing
        // window; it never allocates beyond the ceiling.
        let buffer = LineBuffer::default();
        let mut oversized = vec![b'x'; MAX_BUFFERED_BYTES + 10];
        oversized.extend_from_slice(b"TAIL");
        buffer.push(&oversized);
        let buffered = buffer.inner.lock().unwrap().data.len();
        assert!(buffered <= MAX_BUFFERED_BYTES);
        let mut buf = vec![0_u8; MAX_BUFFERED_BYTES];
        let n = buffer.read(&mut buf).unwrap();
        assert!(buf[..n].ends_with(b"TAIL"));
    }
}
