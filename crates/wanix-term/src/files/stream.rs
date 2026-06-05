use std::sync::Arc;

use wanix_fs::{File, FsError, FsResult, Metadata};

use crate::files::read_from_queue;
use crate::state::{TermResource, TermSide};
use crate::{file_metadata, modes};

const PROGRAM_OUTPUT_CRLF_EXTRA_CAPACITY_DIVISOR: usize = 16;

#[derive(Debug)]
pub(crate) struct TermFile {
    resource: Arc<TermResource>,
    side: TermSide,
    prev_written: Option<u8>,
}

impl TermFile {
    pub(crate) fn new(resource: Arc<TermResource>, side: TermSide) -> Self {
        Self {
            resource,
            side,
            prev_written: None,
        }
    }
}

impl File for TermFile {
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize> {
        self.resource.ensure_open()?;
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
        self.resource.ensure_open()?;
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
        Ok(file_metadata(0, modes::STREAM_FILE))
    }

    fn read_ready(&self) -> FsResult<bool> {
        self.resource.ensure_open()?;
        let io = self
            .resource
            .io
            .lock()
            .map_err(|_| FsError::Other("term resource lock poisoned".to_owned()))?;
        let queue = match self.side {
            TermSide::Data => &io.program_to_data,
            TermSide::Program => &io.data_to_program,
        };
        Ok(!queue.is_empty())
    }
}

fn program_output_bytes(buf: &[u8], prev_written: &mut Option<u8>) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(buf.len() + buf.len() / PROGRAM_OUTPUT_CRLF_EXTRA_CAPACITY_DIVISOR);
    for &byte in buf {
        if byte == b'\n' && *prev_written != Some(b'\r') {
            out.push(b'\r');
        }
        out.push(byte);
        *prev_written = Some(byte);
    }
    out
}
