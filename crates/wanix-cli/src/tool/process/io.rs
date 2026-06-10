//! Process-runner plumbing: the private per-job workdir and bounded capture.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use wanix_job::{ErrorKind, JobError};
use wanix_tool::RunContext;

/// The longest partial stderr line buffered before it is flushed to the
/// events sink anyway (the sink itself is bounded, this only bounds us).
const MAX_PENDING_LINE: usize = 8 * 1024;

/// A fresh private temp directory for one job, removed on drop. The child's
/// cwd, its `{input}` file, and its `{output}` file all live here, so nothing
/// of one run leaks into the next (or into the host's cwd).
#[derive(Debug)]
pub(super) struct Workdir {
    path: PathBuf,
}

impl Workdir {
    /// Creates `$TMPDIR/wanix-tool-<pid>-<seq>-<job>`, owner-private.
    pub(super) fn create(job_id: &str) -> std::io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("wanix-tool-{}-{seq}-{job_id}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// The absolute path of `name` inside the workdir.
    pub(super) fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    /// Reads the child-written `output` file (the `{output}` mapping),
    /// refusing to load more than `cap` bytes into memory.
    pub(super) fn read_output(&self, cap: u64) -> Result<Vec<u8>, JobError> {
        let path = self.file("output");
        let too_large = std::fs::metadata(&path)
            .map(|meta| meta.len() > cap)
            .unwrap_or(false);
        if too_large {
            return Err(JobError::new(
                ErrorKind::RunnerFailed,
                format!("output file exceeded maxOutBytes ({cap})"),
            ));
        }
        std::fs::read(&path).map_err(|error| {
            JobError::new(
                ErrorKind::RunnerFailed,
                format!("process exited 0 but its output file is unreadable: {error}"),
            )
        })
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Drains `reader` to EOF, keeping at most `cap` captured bytes. Once the cap
/// is crossed, `full` is raised (the supervisor kills the child) and further
/// bytes are discarded — the drain continues so the child never blocks on a
/// full pipe before the kill lands. With an `events` context, every complete
/// line is streamed to the job's events sink as it arrives.
pub(super) fn capture_capped(
    mut reader: impl Read,
    cap: u64,
    full: &AtomicBool,
    events: Option<&RunContext>,
) -> Vec<u8> {
    let cap = usize::try_from(cap).unwrap_or(usize::MAX);
    let mut captured = Vec::new();
    let mut pending = Vec::new();
    let mut buf = [0u8; 8 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let chunk = &buf[..n];
        if let Some(ctx) = events {
            stream_lines(ctx, &mut pending, chunk);
        }
        if !full.load(Ordering::SeqCst) {
            let room = cap - captured.len();
            captured.extend_from_slice(&chunk[..n.min(room)]);
            if n > room {
                full.store(true, Ordering::SeqCst);
            }
        }
    }
    if let Some(ctx) = events
        && !pending.is_empty()
    {
        pending.push(b'\n');
        ctx.progress(&pending);
    }
    captured
}

/// Buffers `chunk` and pushes each complete (newline-terminated) line to the
/// events sink; an over-long partial line is flushed early so `pending` stays
/// bounded.
fn stream_lines(ctx: &RunContext, pending: &mut Vec<u8>, chunk: &[u8]) {
    pending.extend_from_slice(chunk);
    while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = pending.drain(..=pos).collect();
        ctx.progress(&line);
    }
    if pending.len() > MAX_PENDING_LINE {
        pending.push(b'\n');
        ctx.progress(pending);
        pending.clear();
    }
}
