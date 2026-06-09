use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use wanix_fs::{FsError, FsResult};

use crate::PipeCapacity;

/// Worst-case wakeup interval for a blocked read or write.
///
/// A read blocks until bytes arrive or all writers close; a write blocks until
/// the reader frees room. Every state change (`write`, `read`, `close_writer`)
/// `notify_all`s, so this timeout only bounds the case where a notification is
/// missed; it is a periodic re-check, NOT an end-of-file or would-block signal.
/// Returning at a timeout would make an empty-but-open pipe indistinguishable
/// from EOF (both look like `Ok(0)` to `fd_read`), so the read loop must only
/// ever return `Ok(0)` once the last writer has closed.
const RECHECK_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct PipeBuffer {
    data: VecDeque<u8>,
    writers: usize,
    readers: usize,
    /// Whether a reader end has ever been opened: a write after the last
    /// reader closed is a broken pipe, but a write before any reader arrives
    /// just buffers (or blocks) — openings are not synchronized.
    reader_seen: bool,
}

/// A unidirectional in-memory byte channel shared by one pipe's reader and
/// writer ends. Bytes written by writers are buffered until read; the reader
/// observes end-of-file only after every writer end has been dropped.
///
/// A bounded channel (the default; see [`PipeCapacity`]) gives Plan 9 pipe
/// semantics for concurrent pipeline stages: a writer blocks while the buffer
/// is full, so a fast producer is back-pressured by its consumer instead of
/// buffering its whole output in memory.
///
/// Lock discipline: blocking happens only inside `Condvar::wait_timeout`,
/// which releases the buffer mutex while parked, and channel operations never
/// call into another filesystem — so a parked reader or writer can never hold
/// a lock another thread needs (the no-lock-across-filesystem-call rule).
pub(crate) struct PipeChannel {
    capacity: Option<NonZeroUsize>,
    buffer: Mutex<PipeBuffer>,
    signal: Condvar,
}

impl PipeChannel {
    pub(crate) fn new(capacity: PipeCapacity) -> Self {
        Self {
            capacity: match capacity {
                PipeCapacity::Bounded(bytes) => Some(bytes),
                PipeCapacity::Unbounded => None,
            },
            buffer: Mutex::new(PipeBuffer::default()),
            signal: Condvar::new(),
        }
    }

    fn lock(&self) -> FsResult<std::sync::MutexGuard<'_, PipeBuffer>> {
        self.buffer
            .lock()
            .map_err(|_| FsError::Other("pipe channel lock poisoned".to_owned()))
    }

    fn wait<'a>(
        &self,
        buffer: std::sync::MutexGuard<'a, PipeBuffer>,
    ) -> FsResult<std::sync::MutexGuard<'a, PipeBuffer>> {
        self.signal
            .wait_timeout(buffer, RECHECK_INTERVAL)
            .map(|(next, _timed_out)| next)
            .map_err(|_| FsError::Other("pipe channel wait poisoned".to_owned()))
    }

    /// Room left before the buffer is full; `buf_len` stands in for "unbounded".
    fn room(&self, buffered: usize, buf_len: usize) -> usize {
        match self.capacity {
            Some(capacity) => capacity.get().saturating_sub(buffered),
            None => buf_len,
        }
    }

    /// Registers a writer end. Balanced by [`Self::close_writer`].
    pub(crate) fn add_writer(&self) -> FsResult<()> {
        let mut buffer = self.lock()?;
        buffer.writers = buffer.writers.saturating_add(1);
        Ok(())
    }

    /// Registers a reader end. Balanced by [`Self::close_reader`].
    pub(crate) fn add_reader(&self) -> FsResult<()> {
        let mut buffer = self.lock()?;
        buffer.readers = buffer.readers.saturating_add(1);
        buffer.reader_seen = true;
        Ok(())
    }

    /// Drops a reader end, waking blocked writers so they can observe a broken
    /// pipe once the last reader is gone (instead of blocking forever on a
    /// full buffer nothing will ever drain).
    pub(crate) fn close_reader(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.readers = buffer.readers.saturating_sub(1);
        }
        self.signal.notify_all();
    }

    /// Drops a writer end, waking any reader so it can observe EOF once the
    /// last writer is gone.
    pub(crate) fn close_writer(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.writers = buffer.writers.saturating_sub(1);
        }
        self.signal.notify_all();
    }

    /// Appends bytes and wakes a blocked reader, blocking while the buffer is
    /// full (bounded channels only). May write fewer bytes than asked (a short
    /// write) when the buffer has some room but not enough; callers loop.
    ///
    /// A write after the last reader end has closed fails (Unix `EPIPE`): the
    /// bytes can never be drained, so blocking would hang the producer — and
    /// whoever waits on the producer — forever. A write before any reader has
    /// opened simply buffers or blocks; consumers may attach late.
    pub(crate) fn write(&self, buf: &[u8]) -> FsResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut buffer = self.lock()?;
        loop {
            if buffer.reader_seen && buffer.readers == 0 {
                return Err(FsError::Other(
                    "pipe write after the last reader closed (broken pipe)".to_owned(),
                ));
            }
            let room = self.room(buffer.data.len(), buf.len());
            if room > 0 {
                let len = buf.len().min(room);
                buffer.data.extend(buf[..len].iter().copied());
                drop(buffer);
                self.signal.notify_all();
                return Ok(len);
            }
            buffer = self.wait(buffer)?;
        }
    }

    /// Drains buffered bytes, blocking until data arrives or every writer has
    /// closed. Returns `Ok(0)` ONLY at end-of-file (no buffered data and no
    /// remaining writers); a wait timeout simply re-checks rather than reporting
    /// EOF. Draining wakes writers blocked on a full buffer.
    pub(crate) fn read(&self, buf: &mut [u8]) -> FsResult<usize> {
        let mut buffer = self.lock()?;
        loop {
            if !buffer.data.is_empty() {
                let len = buffer.data.len().min(buf.len());
                for slot in buf.iter_mut().take(len) {
                    if let Some(byte) = buffer.data.pop_front() {
                        *slot = byte;
                    }
                }
                drop(buffer);
                self.signal.notify_all();
                return Ok(len);
            }
            if buffer.writers == 0 {
                return Ok(0);
            }
            buffer = self.wait(buffer)?;
        }
    }

    /// Reports whether a read would return without blocking: true when bytes are
    /// buffered OR end-of-file has been reached (both are non-blocking reads).
    pub(crate) fn read_ready(&self) -> FsResult<bool> {
        let buffer = self.lock()?;
        Ok(!buffer.data.is_empty() || buffer.writers == 0)
    }

    /// Reports whether a write would return without blocking: true when the
    /// buffer has room (always, for an unbounded channel) or the pipe is
    /// broken (the write returns an immediate error, not a block).
    pub(crate) fn write_ready(&self) -> FsResult<bool> {
        let buffer = self.lock()?;
        let broken = buffer.reader_seen && buffer.readers == 0;
        Ok(broken || self.room(buffer.data.len(), 1) > 0)
    }
}
