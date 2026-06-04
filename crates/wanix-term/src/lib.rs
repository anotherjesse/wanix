//! Terminal device filesystem for Rust Wanix.
//!
//! `TermDevice` provides the first Rust-native version of the Go `#term`
//! service shape: reading `new` allocates a terminal resource, and each
//! resource exposes `id`, `ctl`, `data`, `program`, and `winch` files.

use std::fmt;
use std::sync::{Arc, Mutex};

use wanix_fs::{
    DirEntry, File, FileSystem, FileType, FsError, FsResult, Metadata, NormalizedPath, OpenOptions,
};

mod files;
mod path;
mod state;

use files::{
    BytesFile, ControlFile, NewTermFile, TermFile, WinchFile, access, require_file_open,
    require_read_only,
};
use path::{TermPath, parse_path};
use state::{DeviceState, TermResource, TermSide};

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
        let resource = Arc::new(TermResource::new(id.clone()));
        state.resources.insert(id.clone(), resource);
        Ok(id)
    }

    /// Closes and removes a terminal resource.
    ///
    /// Existing handles become invalid; new opens of the resource fail with
    /// `NotFound`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the resource does not exist or device
    /// state cannot be locked.
    pub fn close(&self, id: &str) -> FsResult<()> {
        let resource = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| FsError::Other("term device lock poisoned".to_owned()))?;
            state.resources.remove(id).ok_or(FsError::NotFound)?
        };
        resource.close()
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
            TermPath::Ctl(id) => Ok(Box::new(ControlFile::new(
                self.clone(),
                id.to_owned(),
                access(options)?,
            ))),
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
            TermPath::Ctl(id) => {
                self.resource(id)?;
                Ok(file_metadata(0, 0o755))
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
                    DirEntry::new("ctl", file_metadata(0, 0o755)),
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

fn directory_metadata() -> Metadata {
    Metadata::new(FileType::Directory, 2, 0o555)
}

fn file_metadata(len: u64, mode: u32) -> Metadata {
    Metadata::new(FileType::File, len, mode)
}

#[cfg(test)]
mod tests;
