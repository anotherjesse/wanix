use std::sync::Arc;

use wanix_fs::{File, FsError, FsResult, Metadata, OpenOptions};

use crate::engine::AgentSession;
use crate::{AgentDevice, file_metadata, modes};

pub(crate) fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read || options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

fn read_from_slice(bytes: &[u8], offset: &mut usize, buf: &mut [u8]) -> FsResult<usize> {
    let remaining = bytes.len().saturating_sub(*offset);
    let len = remaining.min(buf.len());
    buf[..len].copy_from_slice(&bytes[*offset..*offset + len]);
    *offset += len;
    Ok(len)
}

/// `#agent/new`: allocates a session on first read and returns its id.
pub(crate) struct NewAgentFile {
    device: AgentDevice,
    bytes: Option<Vec<u8>>,
    offset: usize,
}

impl NewAgentFile {
    pub(crate) fn new(device: AgentDevice) -> Self {
        Self {
            device,
            bytes: None,
            offset: 0,
        }
    }
}

impl File for NewAgentFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.bytes.is_none() {
            let id = self.device.alloc()?;
            self.bytes = Some(format!("{id}\n").into_bytes());
        }
        let Some(bytes) = self.bytes.as_ref() else {
            return Err(FsError::Other("agent new bytes missing".to_owned()));
        };
        let mut offset = self.offset;
        let read = read_from_slice(bytes, &mut offset, buf)?;
        self.offset = offset;
        Ok(read)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::READ_ONLY_FILE))
    }
}

/// A fixed read-only byte snapshot (`<id>/id`, `<id>/status`).
pub(crate) struct BytesFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl BytesFile {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for BytesFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        read_from_slice(&self.bytes, &mut self.offset, buf)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(
            self.bytes.len() as u64,
            modes::READ_ONLY_FILE,
        ))
    }
}

/// `#agent/<id>/prompt`: each write submits the written text as a new turn.
pub(crate) struct PromptFile {
    session: Arc<dyn AgentSession>,
}

impl PromptFile {
    pub(crate) fn new(session: Arc<dyn AgentSession>) -> Self {
        Self { session }
    }
}

impl File for PromptFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        let prompt = String::from_utf8_lossy(buf);
        self.session.submit(prompt.trim_end())?;
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::STREAM_FILE))
    }
}

/// `#agent/<id>/events`: streams normalized JSONL events, blocking until the
/// next event or the session closes.
pub(crate) struct EventsFile {
    session: Arc<dyn AgentSession>,
}

impl EventsFile {
    pub(crate) fn new(session: Arc<dyn AgentSession>) -> Self {
        Self { session }
    }
}

impl File for EventsFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.session.read_events(buf)
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.session.events_ready()
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::STREAM_FILE))
    }
}

/// `#agent/<id>/ctl`: control verbs. v1 supports `close`.
pub(crate) struct CtlFile {
    device: AgentDevice,
    id: String,
    data: Vec<u8>,
}

impl CtlFile {
    pub(crate) fn new(device: AgentDevice, id: String) -> Self {
        Self {
            device,
            id,
            data: Vec::new(),
        }
    }
}

impl File for CtlFile {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.data.extend_from_slice(buf);
        let command = String::from_utf8_lossy(&self.data).trim().to_owned();
        if command.is_empty() {
            return Ok(buf.len());
        }
        let mut parts = command.split_whitespace();
        let verb = parts.next().unwrap_or_default();
        match verb {
            "close" => self.device.close(&self.id)?,
            "approve" | "deny" => {
                let request_id = parts.next().ok_or_else(|| {
                    FsError::Other("agent ctl: approve/deny require a request id".to_owned())
                })?;
                self.device.resolve(&self.id, request_id, verb)?;
            }
            _ => return Err(FsError::NotSupported),
        }
        self.data.clear();
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, modes::CONTROL_FILE))
    }
}
