//! Terminal device filesystem for Rust Wanix.
//!
//! `TermDevice` provides the first Rust-native version of the Go `#term`
//! service shape: reading `new` allocates a terminal resource, and each
//! resource exposes `id`, `data`, `program`, and `winch` files.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix terminal device filesystem";

/// Filesystem implementing the Rust-native Wanix terminal service.
#[derive(Clone)]
pub struct TermDevice {
    state: Arc<Mutex<DeviceState>>,
}

impl fmt::Debug for TermDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.state.lock() {
            Ok(state) => f
                .debug_struct("TermDevice")
                .field("next_id", &state.next_id)
                .field("resource_count", &state.resources.len())
                .finish(),
            Err(_) => f
                .debug_struct("TermDevice")
                .field("state", &"poisoned")
                .finish(),
        }
    }
}

#[derive(Debug, Default)]
struct DeviceState {
    next_id: u64,
    resources: BTreeMap<String, Arc<TermResource>>,
}

#[derive(Debug)]
struct TermResource {
    id: String,
    io: Mutex<TermIo>,
    winch: Mutex<WinchState>,
}

#[derive(Debug, Default)]
struct TermIo {
    data_to_program: VecDeque<u8>,
    program_to_data: VecDeque<u8>,
}

#[derive(Debug, Default)]
struct WinchState {
    next_subscriber: u64,
    subscribers: BTreeMap<u64, VecDeque<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermSide {
    Data,
    Program,
}

impl Default for TermDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl TermDevice {
    /// Creates an empty terminal device.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(DeviceState::default())),
        }
    }

    /// Allocates a terminal resource and returns its stable resource id.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when device state cannot be locked.
    pub fn alloc(&self) -> FsResult<String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| FsError::Other("term device lock poisoned".to_owned()))?;
        state.next_id = state.next_id.saturating_add(1);
        let id = state.next_id.to_string();
        let resource = Arc::new(TermResource {
            id: id.clone(),
            io: Mutex::new(TermIo::default()),
            winch: Mutex::new(WinchState::default()),
        });
        state.resources.insert(id.clone(), resource);
        Ok(id)
    }

    fn resource(&self, id: &str) -> FsResult<Arc<TermResource>> {
        let state = self
            .state
            .lock()
            .map_err(|_| FsError::Other("term device lock poisoned".to_owned()))?;
        state.resources.get(id).cloned().ok_or(FsError::NotFound)
    }
}

impl FileSystem for TermDevice {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>> {
        match parse_path(path)? {
            TermPath::Root | TermPath::Resource(_) => Err(FsError::IsDirectory),
            TermPath::New => {
                require_read_only(options)?;
                Ok(Box::new(NewTermFile::new(self.clone())))
            }
            TermPath::Id(id) => {
                require_read_only(options)?;
                let resource = self.resource(id)?;
                Ok(Box::new(BytesFile::new(
                    format!("{}\n", resource.id).into_bytes(),
                )))
            }
            TermPath::Data(id) => {
                require_file_open(options)?;
                Ok(Box::new(TermFile::new(self.resource(id)?, TermSide::Data)))
            }
            TermPath::Program(id) => {
                require_file_open(options)?;
                Ok(Box::new(TermFile::new(
                    self.resource(id)?,
                    TermSide::Program,
                )))
            }
            TermPath::Winch(id) => {
                require_file_open(options)?;
                Ok(Box::new(WinchFile::new(
                    self.resource(id)?,
                    options.read,
                    options.write,
                )?))
            }
        }
    }

    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata> {
        match parse_path(path)? {
            TermPath::Root => Ok(directory_metadata()),
            TermPath::New => Ok(file_metadata(0, 0o555)),
            TermPath::Resource(id) => {
                self.resource(id)?;
                Ok(directory_metadata())
            }
            TermPath::Id(id) => {
                let resource = self.resource(id)?;
                Ok(file_metadata((resource.id.len() + 1) as u64, 0o555))
            }
            TermPath::Data(id) | TermPath::Program(id) | TermPath::Winch(id) => {
                self.resource(id)?;
                Ok(file_metadata(0, 0o666))
            }
        }
    }

    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>> {
        match parse_path(path)? {
            TermPath::Root => {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| FsError::Other("term device lock poisoned".to_owned()))?;
                let mut entries = vec![DirEntry::new("new", file_metadata(0, 0o555))];
                entries.extend(
                    state
                        .resources
                        .keys()
                        .map(|id| DirEntry::new(id.clone(), directory_metadata())),
                );
                Ok(entries)
            }
            TermPath::Resource(id) => {
                self.resource(id)?;
                Ok(vec![
                    DirEntry::new("data", file_metadata(0, 0o666)),
                    DirEntry::new("id", file_metadata((id.len() + 1) as u64, 0o555)),
                    DirEntry::new("program", file_metadata(0, 0o666)),
                    DirEntry::new("winch", file_metadata(0, 0o666)),
                ])
            }
            _ => Err(FsError::NotDirectory),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermPath<'a> {
    Root,
    New,
    Resource(&'a str),
    Id(&'a str),
    Data(&'a str),
    Program(&'a str),
    Winch(&'a str),
}

fn parse_path(path: &NormalizedPath) -> FsResult<TermPath<'_>> {
    if path.as_str() == "." {
        return Ok(TermPath::Root);
    }
    let parts = path.as_str().split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["new"] => Ok(TermPath::New),
        [id] => Ok(TermPath::Resource(id)),
        [id, "id"] => Ok(TermPath::Id(id)),
        [id, "data"] => Ok(TermPath::Data(id)),
        [id, "program"] => Ok(TermPath::Program(id)),
        [id, "winch"] => Ok(TermPath::Winch(id)),
        _ => Err(FsError::NotFound),
    }
}

fn require_read_only(options: OpenOptions) -> FsResult<()> {
    if !options.read {
        return Err(FsError::PermissionDenied);
    }
    if options.write || options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

fn require_file_open(options: OpenOptions) -> FsResult<()> {
    if !options.read && !options.write {
        return Err(FsError::PermissionDenied);
    }
    if options.create || options.truncate {
        return Err(FsError::PermissionDenied);
    }
    Ok(())
}

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, 0o555)
}

fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

#[derive(Debug)]
struct NewTermFile {
    device: TermDevice,
    bytes: Option<Vec<u8>>,
    offset: usize,
}

impl NewTermFile {
    fn new(device: TermDevice) -> Self {
        Self {
            device,
            bytes: None,
            offset: 0,
        }
    }
}

impl File for NewTermFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        if self.bytes.is_none() {
            let id = self.device.alloc()?;
            self.bytes = Some(format!("{id}\n").into_bytes());
        }
        read_from_slice(
            self.bytes
                .as_ref()
                .expect("new term bytes are initialized before reading"),
            &mut self.offset,
            buf,
        )
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, 0o555))
    }
}

#[derive(Debug)]
struct BytesFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl BytesFile {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, offset: 0 }
    }
}

impl File for BytesFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        read_from_slice(&self.bytes, &mut self.offset, buf)
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(self.bytes.len() as u64, 0o555))
    }
}

fn read_from_slice(bytes: &[u8], offset: &mut usize, buf: &mut [u8]) -> FsResult<usize> {
    let remaining = bytes.len().saturating_sub(*offset);
    let len = remaining.min(buf.len());
    buf[..len].copy_from_slice(&bytes[*offset..*offset + len]);
    *offset += len;
    Ok(len)
}

#[derive(Debug)]
struct TermFile {
    resource: Arc<TermResource>,
    side: TermSide,
    prev_written: Option<u8>,
}

impl TermFile {
    fn new(resource: Arc<TermResource>, side: TermSide) -> Self {
        Self {
            resource,
            side,
            prev_written: None,
        }
    }
}

impl File for TermFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let mut io = self
            .resource
            .io
            .lock()
            .map_err(|_| FsError::Other("term resource lock poisoned".to_owned()))?;
        let queue = match self.side {
            TermSide::Data => &mut io.program_to_data,
            TermSide::Program => &mut io.data_to_program,
        };
        read_from_queue(queue, buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        let mut io = self
            .resource
            .io
            .lock()
            .map_err(|_| FsError::Other("term resource lock poisoned".to_owned()))?;
        match self.side {
            TermSide::Data => io.data_to_program.extend(buf),
            TermSide::Program => {
                for byte in program_output_bytes(buf, &mut self.prev_written) {
                    io.program_to_data.push_back(byte);
                }
            }
        }
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, 0o666))
    }
}

fn program_output_bytes(buf: &[u8], prev_written: &mut Option<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(buf.len() + buf.len() / 16);
    for &byte in buf {
        if byte == b'\n' && *prev_written != Some(b'\r') {
            out.push(b'\r');
        }
        out.push(byte);
        *prev_written = Some(byte);
    }
    out
}

fn read_from_queue(queue: &mut VecDeque<u8>, buf: &mut [u8]) -> FsResult<usize> {
    let len = queue.len().min(buf.len());
    for slot in buf.iter_mut().take(len) {
        *slot = queue
            .pop_front()
            .expect("queue contains at least len bytes");
    }
    Ok(len)
}

#[derive(Debug)]
struct WinchFile {
    resource: Arc<TermResource>,
    subscriber: Option<u64>,
    writable: bool,
}

impl WinchFile {
    fn new(resource: Arc<TermResource>, readable: bool, writable: bool) -> FsResult<Self> {
        let subscriber = if readable {
            let mut winch = resource
                .winch
                .lock()
                .map_err(|_| FsError::Other("term winch lock poisoned".to_owned()))?;
            winch.next_subscriber = winch.next_subscriber.saturating_add(1);
            let subscriber = winch.next_subscriber;
            winch.subscribers.insert(subscriber, VecDeque::new());
            Some(subscriber)
        } else {
            None
        };
        Ok(Self {
            resource,
            subscriber,
            writable,
        })
    }
}

impl File for WinchFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        let Some(subscriber) = self.subscriber else {
            return Err(FsError::PermissionDenied);
        };
        let mut winch = self
            .resource
            .winch
            .lock()
            .map_err(|_| FsError::Other("term winch lock poisoned".to_owned()))?;
        let queue = winch
            .subscribers
            .get_mut(&subscriber)
            .ok_or(FsError::InvalidFd)?;
        read_from_queue(queue, buf)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        if !self.writable {
            return Err(FsError::PermissionDenied);
        }
        let mut winch = self
            .resource
            .winch
            .lock()
            .map_err(|_| FsError::Other("term winch lock poisoned".to_owned()))?;
        for queue in winch.subscribers.values_mut() {
            queue.extend(buf);
        }
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(file_metadata(0, 0o666))
    }
}

impl Drop for WinchFile {
    fn drop(&mut self) {
        if let Some(subscriber) = self.subscriber
            && let Ok(mut winch) = self.resource.winch.lock()
        {
            winch.subscribers.remove(&subscriber);
        }
    }
}

#[cfg(test)]
mod tests;
