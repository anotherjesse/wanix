//! Chunked-transfer / SSE streaming for gateway device files.
//!
//! A [`super::StaticResponse`] buffers a whole body behind `Content-Length`,
//! which can never carry a never-EOF device file (an AppFS `stream`, a `#plumb`
//! subscription). A [`StreamingResponse`] instead holds the *open file* and
//! pumps it to the socket as `Transfer-Encoding: chunked` frames, flushing
//! after every read so a subscriber sees a line the moment it is published.
//! With `sse` set, the byte stream is re-framed as Server-Sent Events: each
//! newline-terminated input line becomes one `data: <line>\n\n` event, which is
//! what a browser `EventSource` consumes natively.
//!
//! The pump runs on the per-connection thread. While the device reports
//! honest read readiness (`File::read_ready`, which the native mesh wire
//! forwards), the idle wait polls readiness and probes the socket, so an
//! abandoned subscriber (closed tab, `EventSource` reconnect) releases this
//! thread — and, for a mesh-mounted origin, its pinned upstream open-file
//! stream — instead of parking until the next publish. A device without an
//! honest `read_ready` (the always-true trait default) keeps the old
//! behavior: a blocking `File::read`, with disconnect observed at the next
//! failed chunk write.
//!
//! Gateway streams carry no `Access-Control-Allow-Origin`: the WebDoor is an
//! unauthenticated namespace door whose design is one origin per bound name,
//! and a CORS grant here would let any web page the operator's browser visits
//! read live device streams cross-origin (see the `webdoor` trust-boundary
//! docs).

use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

use wanix_fs::File;

use super::super::connection::ServeConnectionError;

/// Bytes pulled per blocking `File::read` while pumping a streamed body.
const STREAM_READ_CHUNK_BYTES: usize = 8 * 1024;

/// How often the idle wait re-probes device readiness and socket liveness.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Read timeout used for the non-blocking socket liveness peek.
const LIVENESS_PEEK_TIMEOUT: Duration = Duration::from_millis(1);

/// An HTTP 200 whose body is pumped live from an open Wanix file.
pub(in crate::serve) struct StreamingResponse {
    /// The `Content-Type` header value sent with the stream.
    pub(in crate::serve) content_type: &'static str,
    /// Re-frame newline-terminated input lines as SSE `data:` events.
    pub(in crate::serve) sse: bool,
    /// The open file the body is pumped from; reads may block between
    /// publishes (never-EOF device contract).
    pub(in crate::serve) file: Box<dyn File>,
}

impl StreamingResponse {
    pub(in crate::serve) fn chunked(file: Box<dyn File>) -> Self {
        Self {
            content_type: "text/plain; charset=utf-8",
            sse: false,
            file,
        }
    }

    pub(in crate::serve) fn sse(file: Box<dyn File>) -> Self {
        Self {
            content_type: "text/event-stream; charset=utf-8",
            sse: true,
            file,
        }
    }
}

/// Writes the streaming headers, then pumps the file to the socket until EOF,
/// a file error, or a client disconnect.
pub(in crate::serve) fn write_streaming_response(
    mut stream: TcpStream,
    mut response: StreamingResponse,
) -> Result<(), ServeConnectionError> {
    let headers = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {}\r\n\
         Transfer-Encoding: chunked\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        response.content_type
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(ServeConnectionError::Io)?;
    pump_file(&mut stream, &mut response)
}

fn pump_file(
    stream: &mut TcpStream,
    response: &mut StreamingResponse,
) -> Result<(), ServeConnectionError> {
    let mut buffer = [0u8; STREAM_READ_CHUNK_BYTES];
    let mut pending_line = Vec::new();
    loop {
        // Idle wait: an abandoned subscriber on a quiet stream must release
        // this thread, not park in the device read until the next publish.
        if !wait_readable_or_disconnect(stream, response.file.as_ref()) {
            return Ok(());
        }
        match response.file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let payload = match response.sse {
                    true => sse_events(&mut pending_line, &buffer[..read]),
                    false => buffer[..read].to_vec(),
                };
                write_chunk(stream, &payload)?;
            }
            // A device read error mid-stream: the headers are already on the
            // wire, so truncate the chunked body (no terminal chunk) — the
            // client observes an aborted transfer, not a clean EOF.
            Err(_) => return Ok(()),
        }
    }
    // Flush a trailing partial line as a final event, then end cleanly.
    if response.sse && !pending_line.is_empty() {
        let last = std::mem::take(&mut pending_line);
        write_chunk(stream, &sse_event(&last))?;
    }
    stream
        .write_all(b"0\r\n\r\n")
        .map_err(ServeConnectionError::Io)?;
    stream.flush().map_err(ServeConnectionError::Io)
}

/// Waits until the device has something to read (or its readiness is
/// unknowable), returning `false` once the client has disconnected.
///
/// `Ok(true)` and `Err` both fall through to the read: an always-true default
/// keeps today's blocking-read behavior, and a probe error is the read's to
/// surface. Only an honest `Ok(false)` (a quiet `LineBuffer` stream, a
/// mesh-imported device via the wire's `ReadReady` op) enters the poll loop.
fn wait_readable_or_disconnect(stream: &TcpStream, file: &dyn File) -> bool {
    loop {
        match file.read_ready() {
            Ok(false) => {}
            Ok(true) | Err(_) => return true,
        }
        if !client_connected(stream) {
            return false;
        }
        std::thread::sleep(IDLE_POLL_INTERVAL);
    }
}

/// Probes the socket without consuming bytes: a clean FIN peeks as `Ok(0)`
/// and a reset as a hard error (disconnected); no data within the tiny
/// timeout means the client is simply quiet (alive).
fn client_connected(stream: &TcpStream) -> bool {
    if stream
        .set_read_timeout(Some(LIVENESS_PEEK_TIMEOUT))
        .is_err()
    {
        return true;
    }
    let mut probe = [0u8; 1];
    let alive = match stream.peek(&mut probe) {
        Ok(0) => false,
        Ok(_) => true,
        Err(error) => matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
    };
    let _ = stream.set_read_timeout(None);
    alive
}

/// Writes one chunked-transfer frame and flushes it so a live subscriber sees
/// the bytes immediately. An empty payload (e.g. SSE still waiting for a full
/// line) writes nothing — `0\r\n\r\n` would terminate the body.
fn write_chunk(stream: &mut TcpStream, payload: &[u8]) -> Result<(), ServeConnectionError> {
    if payload.is_empty() {
        return Ok(());
    }
    let mut frame = format!("{:x}\r\n", payload.len()).into_bytes();
    frame.extend_from_slice(payload);
    frame.extend_from_slice(b"\r\n");
    stream.write_all(&frame).map_err(ServeConnectionError::Io)?;
    stream.flush().map_err(ServeConnectionError::Io)
}

/// Re-frames raw stream bytes into SSE events, one per complete input line.
/// Bytes after the last newline stay buffered in `pending_line` until the next
/// read completes the line (or EOF flushes it).
fn sse_events(pending_line: &mut Vec<u8>, bytes: &[u8]) -> Vec<u8> {
    let mut events = Vec::new();
    for byte in bytes {
        if *byte == b'\n' {
            let line = std::mem::take(pending_line);
            events.extend_from_slice(&sse_event(&line));
        } else {
            pending_line.push(*byte);
        }
    }
    events
}

fn sse_event(line: &[u8]) -> Vec<u8> {
    let trimmed = match line.last() {
        Some(b'\r') => &line[..line.len() - 1],
        _ => line,
    };
    let mut event = b"data: ".to_vec();
    event.extend_from_slice(trimmed);
    event.extend_from_slice(b"\n\n");
    event
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    use wanix_fs::{File, FileType, FsResult, Metadata};

    use super::wait_readable_or_disconnect;

    /// A quiet never-EOF device with honest readiness; `read` must never run.
    struct QuietDevice;

    impl File for QuietDevice {
        fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
            unreachable!("a not-ready device must not be read")
        }

        fn read_ready(&self) -> FsResult<bool> {
            Ok(false)
        }

        fn metadata(&self) -> FsResult<Metadata> {
            Ok(Metadata::new(FileType::File, 0, 0o444))
        }
    }

    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    #[test]
    fn abandoned_subscriber_on_quiet_stream_releases_the_pump() {
        let (client, server) = socket_pair();
        drop(client); // The browser closed the tab; the stream stays quiet.

        let started = Instant::now();
        assert!(!wait_readable_or_disconnect(&server, &QuietDevice));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "disconnect must be observed promptly, not at the next publish"
        );
    }

    #[test]
    fn live_quiet_subscriber_keeps_the_wait_alive_until_readable() {
        let (client, server) = socket_pair();
        let waiter = std::thread::spawn(move || {
            struct ReadyLater(Instant);
            impl File for ReadyLater {
                fn read(&mut self, _buf: &mut [u8]) -> FsResult<usize> {
                    unreachable!()
                }
                fn read_ready(&self) -> FsResult<bool> {
                    Ok(self.0.elapsed() > Duration::from_millis(300))
                }
                fn metadata(&self) -> FsResult<Metadata> {
                    Ok(Metadata::new(FileType::File, 0, 0o444))
                }
            }
            wait_readable_or_disconnect(&server, &ReadyLater(Instant::now()))
        });
        let readable = waiter.join().unwrap();
        assert!(readable, "a live client waits through quiet periods");
        drop(client);
    }
}
