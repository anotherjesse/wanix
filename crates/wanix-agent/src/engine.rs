use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use wanix_fs::{FsError, FsResult};

/// Re-check interval for a blocking events read. A push always notifies, so
/// this only bounds a missed wakeup; it is never an end-of-stream signal.
const EVENT_RECHECK_INTERVAL: Duration = Duration::from_millis(50);

/// Creates an LLM-backed agent session.
///
/// Implementations bridge a concrete engine (a deterministic fake, or a real
/// `codex app-server` subprocess) to the Wanix `#agent` device.
pub trait AgentEngine: Send + Sync {
    /// Starts a new agent session.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the session cannot be started.
    fn start_session(&self) -> FsResult<Arc<dyn AgentSession>>;

    /// A short label for the engine (e.g. `"fake"`, `"codex"`), used by status.
    fn describe(&self) -> &str;
}

/// A live agent session: accepts prompts and produces a normalized event stream.
pub trait AgentSession: Send + Sync {
    /// Submits a user prompt as a new turn.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the turn cannot be submitted.
    fn submit(&self, prompt: &str) -> FsResult<()>;

    /// Reads normalized JSONL event bytes, blocking until data arrives or the
    /// session closes (end-of-stream, `Ok(0)`).
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the stream cannot be read.
    fn read_events(&self, buf: &mut [u8]) -> FsResult<usize>;

    /// Whether an events read would return without blocking (data buffered or
    /// the session has closed).
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when readiness cannot be determined.
    fn events_ready(&self) -> FsResult<bool>;

    /// A human-readable one-line status (engine, turn count, state).
    fn status(&self) -> String;

    /// Returns currently-open approval requests as a JSON array (one object per
    /// request). A powerful action (running a command, editing a file) parks
    /// here until a human resolves it; the default is none.
    fn pending(&self) -> String {
        "[]".to_owned()
    }

    /// Resolves a parked approval request. `decision` is `approve` or `deny`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when no such request is open.
    fn resolve(&self, _request_id: &str, _decision: &str) -> FsResult<()> {
        Err(FsError::NotFound)
    }

    /// Closes the session, ending the event stream and releasing the engine.
    fn close(&self);
}

/// A blocking, in-memory JSONL event stream shared by an engine and the
/// `#agent/<id>/events` file. Engines `push_line` normalized events; the device
/// drains them, observing end-of-stream once the session is closed.
pub struct EventStream {
    buffer: Mutex<EventBuffer>,
    signal: Condvar,
}

#[derive(Default)]
struct EventBuffer {
    data: VecDeque<u8>,
    closed: bool,
}

impl Default for EventStream {
    fn default() -> Self {
        Self::new()
    }
}

impl EventStream {
    /// Creates an empty, open event stream.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Mutex::new(EventBuffer::default()),
            signal: Condvar::new(),
        }
    }

    fn lock(&self) -> FsResult<std::sync::MutexGuard<'_, EventBuffer>> {
        self.buffer
            .lock()
            .map_err(|_| FsError::Other("agent event stream lock poisoned".to_owned()))
    }

    /// Appends one normalized event line (a newline is added) and wakes readers.
    pub fn push_line(&self, line: &str) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.data.extend(line.as_bytes());
            buffer.data.push_back(b'\n');
        }
        self.signal.notify_all();
    }

    /// Marks the stream closed so a blocked read observes end-of-stream.
    pub fn close(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.closed = true;
        }
        self.signal.notify_all();
    }

    /// Drains buffered bytes, blocking until data arrives or the stream closes.
    /// Returns `Ok(0)` only at end-of-stream (no data and closed).
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the lock is poisoned.
    pub fn read(&self, buf: &mut [u8]) -> FsResult<usize> {
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
            if buffer.closed {
                return Ok(0);
            }
            let (next, _timed_out) = self
                .signal
                .wait_timeout(buffer, EVENT_RECHECK_INTERVAL)
                .map_err(|_| FsError::Other("agent event stream wait poisoned".to_owned()))?;
            buffer = next;
        }
    }

    /// Whether a read would return without blocking (data buffered or closed).
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the lock is poisoned.
    pub fn read_ready(&self) -> FsResult<bool> {
        let buffer = self.lock()?;
        Ok(!buffer.data.is_empty() || buffer.closed)
    }
}
