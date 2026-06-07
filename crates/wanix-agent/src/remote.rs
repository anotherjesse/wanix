//! [`RemoteEngine`]: an [`AgentEngine`] that proxies to a remote `#agent` device.
//!
//! The mesh's unifying insight is that every remote capability is a
//! [`wanix_fs::FileSystem`] reachable at `/n/<node>/...`. An imported `#agent`
//! is no exception: its `new`/`<id>/prompt`/`<id>/events`/`<id>/reply`/`<id>/ctl`
//! files are ordinary files on the importing node. `RemoteEngine` operates them
//! through a plain `FileSystem` handle, so a [`RouterEngine`](crate::RouterEngine)
//! can dispatch a session to another node's agent with no transport coupling: the
//! same code drives a loopback `MemFs`-backed `#agent`, a TCP-mounted one, or a
//! QUIC-imported one.
//!
//! # Streaming reads and the import deadlock
//!
//! `<id>/events` and `<id>/reply` are blocking, near-never-EOF reads. When the
//! backing `FileSystem` is a mesh import over one serial 9P stream, a blocking
//! read on `events` would freeze every other operation on that stream — the
//! head-of-line deadlock the blueprint calls out. The fix lives at the import
//! layer (one QUIC bidi stream per blocking open-file); `RemoteEngine` only opens
//! the file through `FileSystem::open` and never assumes the read is cheap.

use std::sync::Arc;

use wanix_fs::{File, FileSystem, FsError, FsResult, NormalizedPath, OpenOptions};

use crate::engine::{AgentEngine, AgentSession};

/// An [`AgentEngine`] backed by a remote `#agent` device exposed as a filesystem.
///
/// Holds the filesystem the remote device is reachable through and the base path
/// of that device within it (for an import mounted at `/n/A`, the base is
/// `n/A/#agent`). Each [`Self::start_session`] reads the device's `new` file to
/// allocate a remote session and returns a [`RemoteSession`] bound to its id.
pub struct RemoteEngine {
    fs: Arc<dyn FileSystem>,
    base: String,
    label: String,
}

impl RemoteEngine {
    /// Builds an engine proxying to the `#agent` device at `base` within `fs`.
    ///
    /// `base` is the device's path inside `fs` with no trailing slash (e.g.
    /// `n/A/#agent`). `label` is a short engine name surfaced by status.
    #[must_use]
    pub fn new(fs: Arc<dyn FileSystem>, base: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            fs,
            base: base.into(),
            label: label.into(),
        }
    }
}

impl AgentEngine for RemoteEngine {
    fn start_session(&self) -> FsResult<Arc<dyn AgentSession>> {
        // Reading `new` on the remote device allocates a session and returns its
        // id, exactly as a local `#agent/new` read does.
        let id = read_to_string(&self.fs, &join(&self.base, "new"))?
            .trim()
            .to_owned();
        if id.is_empty() {
            return Err(FsError::Other(
                "remote #agent/new returned an empty session id".to_owned(),
            ));
        }
        Ok(Arc::new(RemoteSession::new(
            Arc::clone(&self.fs),
            format!("{}/{id}", self.base),
        )))
    }

    fn describe(&self) -> &str {
        &self.label
    }
}

/// One remote agent session: operates a remote `#agent/<id>/…` file set.
pub struct RemoteSession {
    fs: Arc<dyn FileSystem>,
    dir: String,
}

impl RemoteSession {
    /// Binds a session to the remote `#agent/<id>` directory at `dir` in `fs`.
    fn new(fs: Arc<dyn FileSystem>, dir: String) -> Self {
        Self { fs, dir }
    }

    /// Opens `<dir>/<leaf>` with `options` on the remote filesystem.
    fn open(&self, leaf: &str, options: OpenOptions) -> FsResult<Box<dyn File>> {
        let path = NormalizedPath::new(join(&self.dir, leaf))?;
        self.fs.open(&path, options)
    }

    /// Reads a whole snapshot file (`status`, `pending`, `reply`) to a string.
    fn read_snapshot(&self, leaf: &str) -> FsResult<String> {
        read_to_string(&self.fs, &join(&self.dir, leaf))
    }

    /// Writes `bytes` to a write-only control/prompt file in one open.
    fn write_all_to(&self, leaf: &str, bytes: &[u8]) -> FsResult<()> {
        let mut file = self.open(
            leaf,
            OpenOptions {
                write: true,
                ..OpenOptions::default()
            },
        )?;
        let mut written = 0;
        while written < bytes.len() {
            let n = file.write(&bytes[written..])?;
            if n == 0 {
                return Err(FsError::Other(format!(
                    "remote #agent {leaf} accepted no bytes"
                )));
            }
            written += n;
        }
        Ok(())
    }
}

impl AgentSession for RemoteSession {
    fn submit(&self, prompt: &str) -> FsResult<()> {
        self.write_all_to("prompt", prompt.as_bytes())
    }

    fn read_events(&self, buf: &mut [u8]) -> FsResult<usize> {
        // Open the events stream lazily per read call. Each open is a fresh remote
        // handle; the import layer gives blocking opens their own bidi stream so
        // this read cannot freeze the rest of the connection. Re-opening per read
        // restarts the stream, so callers that need a continuous tail keep one
        // handle — which the device's `EventsFile` does by holding the session.
        let mut file = self.open("events", OpenOptions::read())?;
        file.read(buf)
    }

    fn events_ready(&self) -> FsResult<bool> {
        // A remote stream's readiness is not observable without opening it; report
        // ready so the device drains it (the open+read then blocks remotely).
        Ok(true)
    }

    fn status(&self) -> String {
        self.read_snapshot("status")
            .map(|s| s.trim_end().to_owned())
            .unwrap_or_else(|err| format!("remote unreachable: {err}"))
    }

    fn pending(&self) -> String {
        self.read_snapshot("pending")
            .map(|s| s.trim_end().to_owned())
            .unwrap_or_else(|_| "[]".to_owned())
    }

    fn resolve(&self, request_id: &str, decision: &str) -> FsResult<()> {
        self.write_all_to("ctl", format!("{decision} {request_id}").as_bytes())
    }

    fn wait_reply(&self) -> FsResult<String> {
        Ok(self.read_snapshot("reply")?.trim_end().to_owned())
    }

    fn close(&self) {
        // Best-effort: a closed transport need not error a close.
        let _ = self.write_all_to("ctl", b"close");
    }
}

/// Joins a base path and a leaf with a single `/`, with no normalization.
fn join(base: &str, leaf: &str) -> String {
    format!("{base}/{leaf}")
}

/// Opens `path` on `fs` read-only and drains it to a `String`, bounded.
fn read_to_string(fs: &Arc<dyn FileSystem>, path: &str) -> FsResult<String> {
    let normalized = NormalizedPath::new(path)?;
    let mut file = fs.open(&normalized, OpenOptions::read())?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        // A snapshot file is small; refuse an unbounded remote stream.
        if bytes.len() > 1 << 20 {
            return Err(FsError::Other(format!(
                "remote #agent file {path} exceeded 1 MiB"
            )));
        }
    }
    String::from_utf8(bytes).map_err(|err| FsError::Other(err.to_string()))
}

#[cfg(test)]
mod tests;
