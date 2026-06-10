//! [`LineBuffer`]: a bounded, lossy, blocking byte buffer for one stream
//! subscription.
//!
//! The shared subscription-buffer discipline used by `#plumb` `recv` streams,
//! AppFS stream files, host-side guest stderr captures, and job-protocol
//! `events` streams: a push always notifies, a read blocks on a condvar with a
//! periodic re-check (to bound a missed wakeup), and `Ok(0)` is returned only
//! once the buffer is permanently closed — which makes a live stream read
//! honestly never-EOF while its producer owns the buffer.
//!
//! The buffer is bounded and drop-oldest: a slow or absent reader must not let
//! a producer grow memory without bound, so once the per-buffer ceiling is
//! reached the oldest bytes are evicted to make room. Pick the ceiling per use
//! with [`LineBuffer::bounded`]; [`LineBuffer::default`] uses
//! [`DEFAULT_MAX_BUFFERED_BYTES`].

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::{FsError, FsResult};

/// Worst-case re-check interval for a blocked read. A push always notifies, so
/// this only bounds a missed wakeup; it is never an end-of-stream signal.
const READ_RECHECK_INTERVAL: Duration = Duration::from_millis(50);

/// The [`LineBuffer::default`] ceiling: 1 MiB, a useful backlog for line
/// streams while keeping a hard per-subscriber memory cap.
pub const DEFAULT_MAX_BUFFERED_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct Inner {
    data: VecDeque<u8>,
    closed: bool,
}

/// A bounded, lossy, blocking byte buffer: a producer pushes lines/bytes into
/// it and one subscriber drains it, blocking until data arrives or the buffer
/// is permanently closed.
pub struct LineBuffer {
    inner: Mutex<Inner>,
    signal: Condvar,
    max_buffered: usize,
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::bounded(DEFAULT_MAX_BUFFERED_BYTES)
    }
}

impl std::fmt::Debug for LineBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LineBuffer")
            .field("max_buffered", &self.max_buffered)
            .finish_non_exhaustive()
    }
}

impl LineBuffer {
    /// A buffer that holds at most `max_buffered` bytes; once full, the oldest
    /// buffered bytes are dropped to make room (the explicit lossy slow-reader
    /// policy every subscription stream shares).
    #[must_use]
    pub fn bounded(max_buffered: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            signal: Condvar::new(),
            max_buffered,
        }
    }

    fn lock(&self) -> FsResult<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| FsError::Other("line buffer lock poisoned".to_owned()))
    }

    /// Appends produced bytes and wakes a blocked reader.
    ///
    /// Enforces the buffer's ceiling: if appending `bytes` would exceed it,
    /// the oldest buffered bytes are dropped to make room first, so a slow
    /// reader never grows the buffer without bound and always sees the most
    /// recent backlog. A single push larger than the whole ceiling keeps only
    /// its trailing window.
    pub fn push(&self, bytes: &[u8]) {
        if let Ok(mut inner) = self.inner.lock() {
            // Drop the oldest bytes until the new data fits under the ceiling.
            let incoming = bytes.len().min(self.max_buffered);
            while inner.data.len() + incoming > self.max_buffered {
                let overflow = inner.data.len() + incoming - self.max_buffered;
                let drop = overflow.min(inner.data.len());
                if drop == 0 {
                    break;
                }
                inner.data.drain(..drop);
            }
            // If the push itself is larger than the ceiling, keep only its tail.
            let start = bytes.len().saturating_sub(self.max_buffered);
            inner.data.extend(bytes[start..].iter().copied());
        }
        self.signal.notify_all();
    }

    /// Marks the buffer permanently closed so a blocked reader observes EOF
    /// once the remaining bytes drain. Stream files are never-EOF while their
    /// producer lives; this is the producer-side teardown releasing readers.
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
                .map_err(|_| FsError::Other("line buffer wait poisoned".to_owned()))?;
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

    const CEILING: usize = 4 * 1024;

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
    fn push_caps_total_buffered_bytes_against_a_flood() {
        // A producer floods an idle subscriber with far more than the ceiling.
        // The buffer must never exceed it: oldest bytes are dropped.
        let buffer = LineBuffer::bounded(CEILING);
        let chunk = vec![b'a'; CEILING / 4];
        for _ in 0..1000 {
            buffer.push(&chunk);
        }
        let buffered = buffer.inner.lock().unwrap().data.len();
        assert!(
            buffered <= CEILING,
            "buffer grew to {buffered} bytes, above the {CEILING}-byte ceiling"
        );
    }

    #[test]
    fn push_drops_oldest_so_newest_survives() {
        // When the ceiling is reached, the newest bytes are retained and the
        // oldest are evicted: a draining reader sees recent data, not stale.
        let buffer = LineBuffer::bounded(CEILING);
        let filler = vec![b'o'; CEILING];
        buffer.push(&filler);
        buffer.push(b"NEWEST");
        let mut buf = vec![0_u8; CEILING];
        let mut drained = Vec::new();
        while !drained.ends_with(b"NEWEST") {
            let n = buffer.read(&mut buf).unwrap();
            assert!(n > 0, "drained everything without finding the newest push");
            drained.extend_from_slice(&buf[..n]);
        }
        assert!(drained.len() <= CEILING);
    }

    #[test]
    fn push_larger_than_ceiling_keeps_only_its_tail() {
        // A single push bigger than the whole buffer keeps only its trailing
        // window; it never allocates beyond the ceiling.
        let buffer = LineBuffer::bounded(CEILING);
        let mut oversized = vec![b'x'; CEILING + 10];
        oversized.extend_from_slice(b"TAIL");
        buffer.push(&oversized);
        let buffered = buffer.inner.lock().unwrap().data.len();
        assert!(buffered <= CEILING);
        let mut buf = vec![0_u8; CEILING];
        let n = buffer.read(&mut buf).unwrap();
        assert!(buf[..n].ends_with(b"TAIL"));
    }
}
