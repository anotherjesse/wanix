use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use wanix_fs::{FsError, FsResult};

/// Worst-case wakeup interval for a blocking read.
///
/// A read blocks until bytes arrive or all writers close, but a write always
/// `notify_all`s, so this timeout only bounds the case where a notification is
/// missed; it is a periodic re-check, NOT an end-of-file signal. Returning at a
/// timeout would make an empty-but-open pipe indistinguishable from EOF (both
/// look like `Ok(0)` to `fd_read`), so the read loop must only ever return
/// `Ok(0)` once the last writer has closed.
const READ_RECHECK_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct PipeBuffer {
    data: VecDeque<u8>,
    writers: usize,
}

/// A unidirectional in-memory byte channel shared by one pipe's reader and
/// writer ends. Bytes written by writers are buffered until read; the reader
/// observes end-of-file only after every writer end has been dropped.
pub(crate) struct PipeChannel {
    buffer: Mutex<PipeBuffer>,
    signal: Condvar,
}

impl PipeChannel {
    pub(crate) fn new() -> Self {
        Self {
            buffer: Mutex::new(PipeBuffer::default()),
            signal: Condvar::new(),
        }
    }

    fn lock(&self) -> FsResult<std::sync::MutexGuard<'_, PipeBuffer>> {
        self.buffer
            .lock()
            .map_err(|_| FsError::Other("pipe channel lock poisoned".to_owned()))
    }

    /// Registers a writer end. Balanced by [`Self::close_writer`].
    pub(crate) fn add_writer(&self) -> FsResult<()> {
        let mut buffer = self.lock()?;
        buffer.writers = buffer.writers.saturating_add(1);
        Ok(())
    }

    /// Drops a writer end, waking any reader so it can observe EOF once the
    /// last writer is gone.
    pub(crate) fn close_writer(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.writers = buffer.writers.saturating_sub(1);
        }
        self.signal.notify_all();
    }

    /// Appends bytes and wakes a blocked reader.
    pub(crate) fn write(&self, buf: &[u8]) -> FsResult<usize> {
        {
            let mut buffer = self.lock()?;
            buffer.data.extend(buf.iter().copied());
        }
        self.signal.notify_all();
        Ok(buf.len())
    }

    /// Drains buffered bytes, blocking until data arrives or every writer has
    /// closed. Returns `Ok(0)` ONLY at end-of-file (no buffered data and no
    /// remaining writers); a wait timeout simply re-checks rather than reporting
    /// EOF.
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
                return Ok(len);
            }
            if buffer.writers == 0 {
                return Ok(0);
            }
            let (next, _timed_out) = self
                .signal
                .wait_timeout(buffer, READ_RECHECK_INTERVAL)
                .map_err(|_| FsError::Other("pipe channel wait poisoned".to_owned()))?;
            buffer = next;
        }
    }

    /// Reports whether a read would return without blocking: true when bytes are
    /// buffered OR end-of-file has been reached (both are non-blocking reads).
    pub(crate) fn read_ready(&self) -> FsResult<bool> {
        let buffer = self.lock()?;
        Ok(!buffer.data.is_empty() || buffer.writers == 0)
    }
}
