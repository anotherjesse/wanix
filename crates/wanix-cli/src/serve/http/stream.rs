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
//! The pump runs on the per-connection thread and blocks in `File::read`
//! between publishes — never-EOF is a feature of the device, so the connection
//! stays open until the file EOFs (provider gone / stream closed) or the
//! client disconnects (the next chunk write fails and the pump stops).

use std::io::Write;
use std::net::TcpStream;

use wanix_fs::File;

use super::super::connection::ServeConnectionError;

/// Bytes pulled per blocking `File::read` while pumping a streamed body.
const STREAM_READ_CHUNK_BYTES: usize = 8 * 1024;

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
         Access-Control-Allow-Origin: *\r\n\
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
