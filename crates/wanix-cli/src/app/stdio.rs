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
use std::time::{Duration, Instant};

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
        self.recv_line_by(None)
    }

    fn recv_line_deadline(&mut self, deadline: Duration) -> io::Result<Vec<u8>> {
        self.recv_line_by(Some(Instant::now() + deadline))
    }
}

impl PipeReceiver {
    /// How often the bounded receive re-probes pipe readiness.
    const READY_POLL_INTERVAL: Duration = Duration::from_millis(10);

    fn recv_line_by(&mut self, deadline: Option<Instant>) -> io::Result<Vec<u8>> {
        loop {
            if let Some(position) = self.pending.iter().position(|&byte| byte == b'\n') {
                return Ok(self.pending.drain(..=position).collect());
            }
            if self.pending.len() > MAX_LINE_LEN {
                // The line is over the ceiling but framing is intact: return
                // an oversized witness (bounding host memory) and discard the
                // rest of the line. The adapter pump then fails the in-flight
                // op with the specific oversize error while the channel stays
                // up (v0.2 op-fatal vs channel-fatal).
                return self.take_oversized_line();
            }
            if let Some(by) = deadline {
                self.wait_ready_until(by)?;
            }
            let fresh = self.fill()?;
            self.pending.extend_from_slice(&fresh);
        }
    }

    /// Polls the pipe's honest `read_ready` (data buffered, or EOF after the
    /// last writer dropped) until `by`, so the bounded receive never enters a
    /// blocking read it cannot come back from.
    fn wait_ready_until(&self, by: Instant) -> io::Result<()> {
        loop {
            if self
                .guest_stdout
                .read_ready()
                .map_err(|err| guest_down(&err))?
            {
                return Ok(());
            }
            if Instant::now() >= by {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "app guest sent nothing before the handshake deadline",
                ));
            }
            std::thread::sleep(Self::READY_POLL_INTERVAL);
        }
    }
    /// Reads one chunk of guest output, mapping EOF to the guest-exit error.
    fn fill(&mut self) -> io::Result<Vec<u8>> {
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
        Ok(buf[..count].to_vec())
    }

    /// Returns the over-ceiling line truncated to an oversized witness
    /// (still over [`MAX_LINE_LEN`], newline-terminated) and discards the
    /// rest of the line; bytes after its newline stay buffered.
    fn take_oversized_line(&mut self) -> io::Result<Vec<u8>> {
        let mut line: Vec<u8> = self.pending.drain(..).collect();
        line.truncate(MAX_LINE_LEN + 1);
        loop {
            let fresh = self.fill()?;
            if let Some(position) = fresh.iter().position(|&byte| byte == b'\n') {
                self.pending.extend_from_slice(&fresh[position + 1..]);
                line.push(b'\n');
                return Ok(line);
            }
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use wanix_appfs::AppReceiver as _;
    use wanix_fs::{FileSystem as _, NormalizedPath, OpenOptions};
    use wanix_pipe::PipeDevice;

    use super::PipeReceiver;

    fn pipe_receiver() -> (PipeReceiver, Box<dyn wanix_fs::File>) {
        let pipes = PipeDevice::new();
        let id = pipes.alloc().unwrap();
        let path = NormalizedPath::new(format!("{id}/data")).unwrap();
        let read_end = pipes.open(&path, OpenOptions::read()).unwrap();
        let write_end = pipes
            .open(
                &path,
                OpenOptions {
                    write: true,
                    ..OpenOptions::default()
                },
            )
            .unwrap();
        (
            PipeReceiver {
                guest_stdout: read_end,
                pending: Vec::new(),
            },
            write_end,
        )
    }

    #[test]
    fn deadline_receive_times_out_on_a_silent_live_guest() {
        // The write end stays open (the guest is alive but says nothing), so
        // only the deadline — never EOF — can end the wait.
        let (mut receiver, _writer) = pipe_receiver();
        let started = Instant::now();
        let error = receiver
            .recv_line_deadline(Duration::from_millis(100))
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the handshake wait must be bounded"
        );
    }

    #[test]
    fn deadline_receive_delivers_a_line_that_arrives_in_time() {
        let (mut receiver, mut writer) = pipe_receiver();
        writer.write(b"{\"hello\":{\"proto\":1}}\n").unwrap();
        let line = receiver.recv_line_deadline(Duration::from_secs(5)).unwrap();
        assert_eq!(line, b"{\"hello\":{\"proto\":1}}\n");
    }
}
