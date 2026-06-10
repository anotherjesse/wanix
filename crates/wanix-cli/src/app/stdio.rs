//! Guest stdio plumbing: the `#pipe`-backed channel halves the adapter
//! transacts over, and the bounded stderr capture.
//!
//! [`PipeSender`] owns the host write end of the guest's stdin pipe (its drop
//! is what EOFs a guest whose service has been torn down); [`PipeReceiver`]
//! owns the host read end of the guest's stdout pipe and is handed to the
//! adapter's pump thread, the single reader of guest output. A broken or
//! EOF'd pipe means the guest is gone; the io error surfaces through the
//! adapter as [`FsError::Unreachable`].

use std::io;
use std::sync::Arc;

use wanix_appfs::{AppReceiver, AppSender, LineBuffer, MAX_LINE_LEN};
use wanix_fs::{File, FileType, FsError, FsResult, Metadata};

pub(crate) struct PipeSender {
    pub(crate) guest_stdin: Box<dyn File>,
}

pub(crate) struct PipeReceiver {
    pub(crate) guest_stdout: Box<dyn File>,
    pub(crate) pending: Vec<u8>,
}

fn guest_down(err: &FsError) -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!("app guest stdio pipe: {err}"),
    )
}

impl AppSender for PipeSender {
    fn send_line(&mut self, line: &[u8]) -> io::Result<()> {
        let mut written = 0;
        while written < line.len() {
            let count = self
                .guest_stdin
                .write(&line[written..])
                .map_err(|err| guest_down(&err))?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "app guest stdin accepted no bytes",
                ));
            }
            written += count;
        }
        Ok(())
    }
}

impl AppReceiver for PipeReceiver {
    fn recv_line(&mut self) -> io::Result<Vec<u8>> {
        loop {
            if let Some(position) = self.pending.iter().position(|&byte| byte == b'\n') {
                return Ok(self.pending.drain(..=position).collect());
            }
            if self.pending.len() > MAX_LINE_LEN {
                return Err(io::Error::other(format!(
                    "app guest emitted over {MAX_LINE_LEN} bytes without a newline"
                )));
            }
            let mut buf = [0u8; 4096];
            let count = self
                .guest_stdout
                .read(&mut buf)
                .map_err(|err| guest_down(&err))?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "app guest exited (its stdout pipe closed)",
                ));
            }
            self.pending.extend_from_slice(&buf[..count]);
        }
    }
}

/// The guest's fd 2: a write-only file feeding a bounded drop-oldest
/// [`LineBuffer`] (the appfs stream-buffer discipline), so a chatty resident
/// guest can never grow serve-process memory without bound. The exit watcher
/// drains the most recent backlog for its log line.
pub(crate) struct StderrCapture {
    buffer: Arc<LineBuffer>,
}

impl StderrCapture {
    pub(crate) fn new(buffer: Arc<LineBuffer>) -> Self {
        Self { buffer }
    }
}

impl File for StderrCapture {
    fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
        Ok(0)
    }

    fn write(&mut self, buf: &[u8]) -> FsResult<usize> {
        self.buffer.push(buf);
        Ok(buf.len())
    }

    fn metadata(&self) -> FsResult<Metadata> {
        Ok(Metadata::new(FileType::File, 0, 0o222))
    }
}

/// Drains whatever the capture currently holds, without blocking.
pub(crate) fn drain_captured(buffer: &LineBuffer) -> String {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 4096];
    while buffer.read_ready().unwrap_or(false) {
        match buffer.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(count) => bytes.extend_from_slice(&buf[..count]),
        }
    }
    String::from_utf8_lossy(&bytes).trim().to_owned()
}
