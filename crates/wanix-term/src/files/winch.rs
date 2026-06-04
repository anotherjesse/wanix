use std::collections::VecDeque;
use std::sync::Arc;

use wanix_fs::{File, FsError, FsResult, Metadata};

use crate::file_metadata;
use crate::files::read_from_queue;
use crate::state::TermResource;

#[derive(Debug)]
pub(crate) struct WinchFile {
    resource: Arc<TermResource>,
    subscriber: Option<u64>,
    writable: bool,
}

impl WinchFile {
    pub(crate) fn new(
        resource: Arc<TermResource>,
        readable: bool,
        writable: bool,
    ) -> FsResult<Self> {
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
        self.resource.ensure_open()?;
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
        self.resource.ensure_open()?;
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

    fn read_ready(&self) -> FsResult<bool> {
        self.resource.ensure_open()?;
        let Some(subscriber) = self.subscriber else {
            return Ok(false);
        };
        let winch = self
            .resource
            .winch
            .lock()
            .map_err(|_| FsError::Other("term winch lock poisoned".to_owned()))?;
        let queue = winch
            .subscribers
            .get(&subscriber)
            .ok_or(FsError::InvalidFd)?;
        Ok(!queue.is_empty())
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
