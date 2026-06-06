//! Agent device filesystem for Rust Wanix.
//!
//! `AgentDevice` exposes an `#agent` service shaped like `#term`: reading `new`
//! allocates an LLM session and returns its id, then each `#agent/<id>/…` file
//! drives it — write a prompt to `prompt`, `cat` the streaming reply from
//! `events`, read `status`, and `close` via `ctl`. The session is produced by a
//! pluggable [`AgentEngine`] (a deterministic [`FakeEngine`] for tests, or a
//! real `codex app-server` bridge), so an LLM becomes something you can
//! `echo`/`cat` in the Plan9 idiom.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

mod codex;
mod engine;
mod exec_server;
mod fake;
mod files;
mod path;

pub use codex::CodexEngine;
pub use engine::{AgentEngine, AgentSession, EventStream};
pub use exec_server::ExecServer;
pub use fake::FakeEngine;

use files::{BytesFile, CtlFile, EventsFile, NewAgentFile, PromptFile, require_read_only};
use path::{AgentPath, parse_path};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix agent device filesystem";

pub(crate) mod modes {
    pub(crate) const READ_ONLY_FILE: u32 = 0o555;
    pub(crate) const STREAM_FILE: u32 = 0o666;
    pub(crate) const CONTROL_FILE: u32 = 0o755;
    pub(crate) const DIRECTORY: u32 = READ_ONLY_FILE;
}

#[derive(Default)]
struct DeviceState {
    next_id: u64,
    sessions: BTreeMap<String, Arc<dyn AgentSession>>,
}

/// Filesystem implementing the Rust-native Wanix agent service.
#[derive(Clone)]
pub struct AgentDevice {
    engine: Arc<dyn AgentEngine>,
    state: Arc<Mutex<DeviceState>>,
}

impl fmt::Debug for AgentDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sessions = self.state.lock().map(|s| s.sessions.len()).ok();
        f.debug_struct("AgentDevice")
            .field("engine", &self.engine.describe())
            .field("sessions", &sessions)
            .finish()
    }
}

impl AgentDevice {
    /// Creates an agent device backed by `engine`.
    #[must_use]
    pub fn new(engine: Arc<dyn AgentEngine>) -> Self {
        Self {
            engine,
            state: Arc::new(Mutex::new(DeviceState::default())),
        }
    }

    /// Allocates a session (starting an engine session) and returns its id.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the engine cannot start a session or
    /// device state cannot be locked.
    pub fn alloc(&self) -> FsResult<String> {
        let session = self.engine.start_session()?;
        let mut state = self.lock_state()?;
        state.next_id = state.next_id.saturating_add(1);
        let id = state.next_id.to_string();
        state.sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// Closes and removes a session.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the session does not exist or device
    /// state cannot be locked.
    pub fn close(&self, id: &str) -> FsResult<()> {
        let session = {
            let mut state = self.lock_state()?;
            state.sessions.remove(id).ok_or(FsError::NotFound)?
        };
        session.close();
        Ok(())
    }

    /// Resolves a parked approval request for session `id`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the session or request does not exist.
    pub fn resolve(&self, id: &str, request_id: &str, decision: &str) -> FsResult<()> {
        self.session(id)?.resolve(request_id, decision)
    }

    fn session(&self, id: &str) -> FsResult<Arc<dyn AgentSession>> {
        let state = self.lock_state()?;
        state.sessions.get(id).cloned().ok_or(FsError::NotFound)
    }

    fn lock_state(&self) -> FsResult<std::sync::MutexGuard<'_, DeviceState>> {
        self.state
            .lock()
            .map_err(|_| FsError::Other("agent device lock poisoned".to_owned()))
    }
}

impl FileSystem for AgentDevice {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        match parse_path(path)? {
            AgentPath::Root | AgentPath::Session(_) => Err(FsError::IsDirectory),
            AgentPath::New => {
                require_read_only(options)?;
                Ok(Box::new(NewAgentFile::new(self.clone())))
            }
            AgentPath::Id(id) => {
                require_read_only(options)?;
                self.session(id)?;
                Ok(Box::new(BytesFile::new(format!("{id}\n").into_bytes())))
            }
            AgentPath::Status(id) => {
                require_read_only(options)?;
                let status = self.session(id)?.status();
                Ok(Box::new(BytesFile::new(format!("{status}\n").into_bytes())))
            }
            AgentPath::Events(id) => {
                require_read_only(options)?;
                Ok(Box::new(EventsFile::new(self.session(id)?)))
            }
            AgentPath::Pending(id) => {
                require_read_only(options)?;
                let pending = self.session(id)?.pending();
                Ok(Box::new(BytesFile::new(
                    format!("{pending}\n").into_bytes(),
                )))
            }
            AgentPath::Prompt(id) => Ok(Box::new(PromptFile::new(self.session(id)?))),
            AgentPath::Ctl(id) => {
                self.session(id)?;
                Ok(Box::new(CtlFile::new(self.clone(), id.to_owned())))
            }
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            AgentPath::Root => Ok(directory_metadata()),
            AgentPath::New => Ok(file_metadata(0, modes::READ_ONLY_FILE)),
            AgentPath::Session(id) => {
                self.session(id)?;
                Ok(directory_metadata())
            }
            AgentPath::Id(id) | AgentPath::Status(id) | AgentPath::Pending(id) => {
                self.session(id)?;
                Ok(file_metadata(0, modes::READ_ONLY_FILE))
            }
            AgentPath::Prompt(id) | AgentPath::Events(id) => {
                self.session(id)?;
                Ok(file_metadata(0, modes::STREAM_FILE))
            }
            AgentPath::Ctl(id) => {
                self.session(id)?;
                Ok(file_metadata(0, modes::CONTROL_FILE))
            }
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match parse_path(path)? {
            AgentPath::Root => {
                let state = self.lock_state()?;
                let mut entries = vec![DirEntry::new(
                    "new",
                    file_metadata(0, modes::READ_ONLY_FILE),
                )];
                entries.extend(
                    state
                        .sessions
                        .keys()
                        .map(|id| DirEntry::new(id.clone(), directory_metadata())),
                );
                Ok(entries)
            }
            AgentPath::Session(id) => {
                self.session(id)?;
                Ok(vec![
                    DirEntry::new("ctl", file_metadata(0, modes::CONTROL_FILE)),
                    DirEntry::new("events", file_metadata(0, modes::STREAM_FILE)),
                    DirEntry::new("id", file_metadata(0, modes::READ_ONLY_FILE)),
                    DirEntry::new("pending", file_metadata(0, modes::READ_ONLY_FILE)),
                    DirEntry::new("prompt", file_metadata(0, modes::STREAM_FILE)),
                    DirEntry::new("status", file_metadata(0, modes::READ_ONLY_FILE)),
                ])
            }
            _ => Err(FsError::NotDirectory),
        }
    }
}

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, modes::DIRECTORY)
}

fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

#[cfg(test)]
mod tests;
