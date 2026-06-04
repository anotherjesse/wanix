use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use wanix_fs::{FsError, FsResult};

#[derive(Debug, Default)]
pub(crate) struct DeviceState {
    pub(crate) next_id: u64,
    pub(crate) resources: BTreeMap<String, std::sync::Arc<TermResource>>,
}

#[derive(Debug)]
pub(crate) struct TermResource {
    pub(crate) id: String,
    pub(crate) io: Mutex<TermIo>,
    pub(crate) winch: Mutex<WinchState>,
    closed: Mutex<bool>,
}

impl TermResource {
    pub(crate) fn new(id: String) -> Self {
        Self {
            id,
            io: Mutex::new(TermIo::default()),
            winch: Mutex::new(WinchState::default()),
            closed: Mutex::new(false),
        }
    }

    pub(crate) fn close(&self) -> FsResult<()> {
        let mut closed = self
            .closed
            .lock()
            .map_err(|_| FsError::Other("term resource close lock poisoned".to_owned()))?;
        *closed = true;
        Ok(())
    }

    pub(crate) fn ensure_open(&self) -> FsResult<()> {
        let closed = self
            .closed
            .lock()
            .map_err(|_| FsError::Other("term resource close lock poisoned".to_owned()))?;
        if *closed {
            Err(FsError::InvalidFd)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct TermIo {
    pub(crate) data_to_program: VecDeque<u8>,
    pub(crate) program_to_data: VecDeque<u8>,
}

#[derive(Debug, Default)]
pub(crate) struct WinchState {
    pub(crate) next_subscriber: u64,
    pub(crate) subscribers: BTreeMap<u64, VecDeque<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TermSide {
    Data,
    Program,
}
