//! The streaming, cancellable interactive `cat`.
//!
//! A terminal-bound `cat` (interactive REPL, stdout inherited) must not
//! collect its source into memory: a never-EOF source (`#pipe/<id>/data`, a
//! device stream) would buffer forever and the user could never get the
//! prompt back. This module streams each source to stdout chunk by chunk,
//! and between chunks waits cancellably ([`NamespaceOps::source_wait_cancellable`],
//! `poll_oneoff` over the source fd and stdin in the WASI backing): Ctrl-C
//! drops the source handle and returns status 130 so the REPL prints a fresh
//! prompt.
//!
//! Pipeline stages keep the collected `cat` builtin (`crate::builtins::cat`):
//! builtin stages exchange bytes through shell memory by design (see
//! `crate::pipeline`), so streaming applies only where the bytes reach the
//! terminal. A backing without streaming support (`source_open` refuses with
//! `Unsupported`) falls back to the collected read, so host fakes keep
//! working unchanged.

use crate::ShellError;
use crate::ns::{InputSource, NamespaceOps, SourceWait};

/// `128 + SIGINT`: the status of a Ctrl-C-cancelled command.
pub(crate) const SIGINT_STATUS: i32 = 130;

const STREAM_CHUNK_BYTES: usize = 8192;

/// What streaming one source ended with.
enum StreamEnd {
    /// End-of-stream (or a reported per-source failure): carry on.
    Done(i32),
    /// Ctrl-C: stop the whole command with [`SIGINT_STATUS`].
    Cancelled,
}

/// Runs an interactive, terminal-bound `cat`: streams the operand files (or
/// the `<` redirect source) to stdout, cancellable from stdin.
pub(crate) fn stream_cat(argv: &[String], stdin: &InputSource, ns: &mut dyn NamespaceOps) -> i32 {
    let mut status = 0;
    if argv.len() == 1 {
        // No operands: stream the `<` redirect; plain inherited stdin has no
        // bytes to copy (matching the collected builtin's behavior).
        return match stdin {
            InputSource::File(path) => match stream_source(path, ns) {
                StreamEnd::Done(code) => code,
                StreamEnd::Cancelled => SIGINT_STATUS,
            },
            InputSource::Inherit | InputSource::Pipe(_) => 0,
        };
    }
    for path in &argv[1..] {
        if path == "-" {
            // `-` names stdin, which carries no buffered bytes here (the
            // collected builtin handles `-` inside pipelines).
            continue;
        }
        match stream_source(path, ns) {
            StreamEnd::Done(code) => status = status.max(code),
            StreamEnd::Cancelled => return SIGINT_STATUS,
        }
    }
    status
}

/// Streams one source to stdout; reports failures like the collected `cat`
/// (named on stderr, non-zero status, never a silent empty success).
fn stream_source(path: &str, ns: &mut dyn NamespaceOps) -> StreamEnd {
    let handle = match ns.source_open(path) {
        Ok(handle) => handle,
        // No streaming support in this backing: collected fallback.
        Err(ShellError::Unsupported(_)) => return collected_fallback(path, ns),
        Err(err) => return report(ns, &err),
    };
    let mut buf = [0u8; STREAM_CHUNK_BYTES];
    loop {
        match ns.source_wait_cancellable(&handle) {
            Ok(SourceWait::Ready) => {}
            Ok(SourceWait::Cancelled) => {
                ns.source_close(handle);
                let _ = ns.write_stdout(b"^C\n");
                return StreamEnd::Cancelled;
            }
            Err(err) => {
                ns.source_close(handle);
                return report(ns, &err);
            }
        }
        match ns.source_read(&handle, &mut buf) {
            Ok(0) => break,
            Ok(read) => {
                if let Err(err) = ns.write_stdout(&buf[..read]) {
                    ns.source_close(handle);
                    return report(ns, &err);
                }
            }
            Err(err) => {
                ns.source_close(handle);
                return report(ns, &err);
            }
        }
    }
    ns.source_close(handle);
    StreamEnd::Done(0)
}

fn collected_fallback(path: &str, ns: &mut dyn NamespaceOps) -> StreamEnd {
    match ns.read_file(path) {
        Ok(bytes) => match ns.write_stdout(&bytes) {
            Ok(()) => StreamEnd::Done(0),
            Err(err) => report(ns, &err),
        },
        Err(err) => report(ns, &err),
    }
}

fn report(ns: &mut dyn NamespaceOps, err: &ShellError) -> StreamEnd {
    let _ = ns.write_stderr(format!("cat: {err}\n").as_bytes());
    StreamEnd::Done(1)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{SIGINT_STATUS, stream_cat};
    use crate::error::ShellResult;
    use crate::ns::{InputSource, NamespaceOps, SourceHandle, SourceWait, SpawnHandle, SpawnSpec};

    /// A streaming-capable fake: one openable source served as scripted
    /// chunks, with a scripted wait outcome per loop turn.
    #[derive(Default)]
    struct StreamNs {
        source_path: String,
        chunks: VecDeque<Vec<u8>>,
        waits: VecDeque<SourceWait>,
        out: Vec<u8>,
        err: Vec<u8>,
        open_handles: usize,
        files: std::collections::HashMap<String, Vec<u8>>,
    }

    impl StreamNs {
        fn with_source(path: &str, chunks: &[&[u8]]) -> Self {
            Self {
                source_path: path.to_owned(),
                chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
                ..Self::default()
            }
        }

        fn out(&self) -> String {
            String::from_utf8_lossy(&self.out).into_owned()
        }
    }

    impl NamespaceOps for StreamNs {
        fn write_stdout(&mut self, bytes: &[u8]) -> ShellResult<()> {
            self.out.extend_from_slice(bytes);
            Ok(())
        }
        fn write_stderr(&mut self, bytes: &[u8]) -> ShellResult<()> {
            self.err.extend_from_slice(bytes);
            Ok(())
        }
        fn read_stdin(&mut self, _buf: &mut [u8]) -> ShellResult<usize> {
            Ok(0)
        }
        fn exists(&self, _path: &str) -> ShellResult<bool> {
            Ok(false)
        }
        fn read_file(&mut self, path: &str) -> ShellResult<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| crate::ShellError::Io(format!("{path}: not found")))
        }
        fn write_file(&mut self, _path: &str, _bytes: &[u8], _append: bool) -> ShellResult<()> {
            Ok(())
        }
        fn pipe_new(&mut self) -> ShellResult<String> {
            Ok("0".into())
        }
        fn pipe_read_all(&mut self, _id: &str) -> ShellResult<Vec<u8>> {
            Ok(Vec::new())
        }
        fn pipe_open_writer(&mut self, _id: &str) -> ShellResult<()> {
            Ok(())
        }
        fn pipe_break_reader(&mut self, _id: &str) -> ShellResult<()> {
            Ok(())
        }
        fn pipe_write_all_and_close(&mut self, _id: &str, _bytes: &[u8]) -> ShellResult<()> {
            Ok(())
        }
        fn spawn_start(&mut self, _spec: &SpawnSpec) -> ShellResult<SpawnHandle> {
            Err(crate::ShellError::Io("no externals".into()))
        }
        fn spawn_wait(&mut self, _handle: &SpawnHandle) -> ShellResult<i32> {
            Err(crate::ShellError::Io("no externals".into()))
        }

        fn source_open(&mut self, path: &str) -> ShellResult<SourceHandle> {
            if path != self.source_path {
                return Err(crate::ShellError::Io(format!("{path}: not found")));
            }
            self.open_handles += 1;
            Ok(SourceHandle::new(path))
        }
        fn source_read(&mut self, _handle: &SourceHandle, buf: &mut [u8]) -> ShellResult<usize> {
            match self.chunks.pop_front() {
                Some(chunk) => {
                    buf[..chunk.len()].copy_from_slice(&chunk);
                    Ok(chunk.len())
                }
                None => Ok(0),
            }
        }
        fn source_close(&mut self, _handle: SourceHandle) {
            self.open_handles -= 1;
        }
        fn source_wait_cancellable(&mut self, _handle: &SourceHandle) -> ShellResult<SourceWait> {
            Ok(self.waits.pop_front().unwrap_or(SourceWait::Ready))
        }
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn streams_operand_chunks_through_to_stdout_until_eof() {
        let mut ns = StreamNs::with_source("notes.txt", &[b"alpha ", b"beta"]);
        let status = stream_cat(&argv(&["cat", "notes.txt"]), &InputSource::Inherit, &mut ns);
        assert_eq!(status, 0);
        assert_eq!(ns.out(), "alpha beta");
        assert_eq!(ns.open_handles, 0, "source handle released at EOF");
    }

    #[test]
    fn streams_the_read_redirect_source_when_there_are_no_operands() {
        let mut ns = StreamNs::with_source("in.txt", &[b"redirected"]);
        let status = stream_cat(
            &argv(&["cat"]),
            &InputSource::File("in.txt".into()),
            &mut ns,
        );
        assert_eq!(status, 0);
        assert_eq!(ns.out(), "redirected");
    }

    #[test]
    fn ctrl_c_mid_stream_drops_the_source_and_reports_130() {
        // Two chunks flow, then the wait observes Ctrl-C: the already-written
        // bytes stay (write-through), the handle drops, status is 130.
        let mut ns = StreamNs::with_source("dev", &[b"one ", b"two ", b"never"]);
        ns.waits = [SourceWait::Ready, SourceWait::Ready, SourceWait::Cancelled]
            .into_iter()
            .collect();
        let status = stream_cat(&argv(&["cat", "dev"]), &InputSource::Inherit, &mut ns);
        assert_eq!(status, SIGINT_STATUS);
        assert_eq!(ns.out(), "one two ^C\n", "chunks were written through");
        assert_eq!(ns.open_handles, 0, "cancel dropped the source handle");
    }

    #[test]
    fn missing_operand_is_a_loud_failure() {
        let mut ns = StreamNs::with_source("present.txt", &[b"x"]);
        let status = stream_cat(
            &argv(&["cat", "missing.txt"]),
            &InputSource::Inherit,
            &mut ns,
        );
        assert_eq!(status, 1);
        assert!(String::from_utf8_lossy(&ns.err).contains("missing.txt"));
    }

    /// A fake with NO streaming support: every `source_*` default applies.
    struct CollectedOnlyNs(StreamNs);

    impl NamespaceOps for CollectedOnlyNs {
        fn write_stdout(&mut self, bytes: &[u8]) -> ShellResult<()> {
            self.0.write_stdout(bytes)
        }
        fn write_stderr(&mut self, bytes: &[u8]) -> ShellResult<()> {
            self.0.write_stderr(bytes)
        }
        fn read_stdin(&mut self, buf: &mut [u8]) -> ShellResult<usize> {
            self.0.read_stdin(buf)
        }
        fn exists(&self, path: &str) -> ShellResult<bool> {
            self.0.exists(path)
        }
        fn read_file(&mut self, path: &str) -> ShellResult<Vec<u8>> {
            self.0.read_file(path)
        }
        fn write_file(&mut self, path: &str, bytes: &[u8], append: bool) -> ShellResult<()> {
            self.0.write_file(path, bytes, append)
        }
        fn pipe_new(&mut self) -> ShellResult<String> {
            self.0.pipe_new()
        }
        fn pipe_read_all(&mut self, id: &str) -> ShellResult<Vec<u8>> {
            self.0.pipe_read_all(id)
        }
        fn pipe_open_writer(&mut self, id: &str) -> ShellResult<()> {
            self.0.pipe_open_writer(id)
        }
        fn pipe_break_reader(&mut self, id: &str) -> ShellResult<()> {
            self.0.pipe_break_reader(id)
        }
        fn pipe_write_all_and_close(&mut self, id: &str, bytes: &[u8]) -> ShellResult<()> {
            self.0.pipe_write_all_and_close(id, bytes)
        }
        fn spawn_start(&mut self, spec: &SpawnSpec) -> ShellResult<SpawnHandle> {
            self.0.spawn_start(spec)
        }
        fn spawn_wait(&mut self, handle: &SpawnHandle) -> ShellResult<i32> {
            self.0.spawn_wait(handle)
        }
    }

    #[test]
    fn backing_without_streaming_support_falls_back_to_collected_read() {
        let mut inner = StreamNs::default();
        inner
            .files
            .insert("plain.txt".to_owned(), b"collected bytes".to_vec());
        let mut ns = CollectedOnlyNs(inner);
        let status = stream_cat(&argv(&["cat", "plain.txt"]), &InputSource::Inherit, &mut ns);
        assert_eq!(status, 0);
        assert_eq!(ns.0.out(), "collected bytes");
    }
}
