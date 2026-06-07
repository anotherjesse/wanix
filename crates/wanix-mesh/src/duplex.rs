//! The synchronous bridge over an async iroh QUIC bidi stream.
//!
//! The 9P core ([`wanix_9p::P9Server::serve_stream`] and
//! [`wanix_9p_client::RemoteFs`]) is strictly synchronous and must never learn
//! about async. iroh's [`SendStream`]/[`RecvStream`] are async. This module is
//! the seam: it drives the async stream halves on a **held** tokio runtime
//! [`Handle`] behind the blocking [`Read`]/[`Write`] contract.
//!
//! Two shapes are provided because the two sides of 9P want different things:
//!
//! - [`BlockingReader`] and [`BlockingWriter`] own one stream half each. The
//!   server's [`wanix_9p::P9Server::serve_stream`] takes a separate `Read` and
//!   `Write`, so the inbound path splits the bidi stream into these two.
//! - [`BlockingDuplex`] owns both halves and is `Read + Write`, satisfying
//!   [`wanix_9p_client::Duplex`] for the outbound [`wanix_9p_client::RemoteFs`].
//!
//! # The runtime-worker hazard
//!
//! `Handle::block_on` panics if called from a thread that is itself a runtime
//! worker. The blueprint flagged this as the concrete bridge hazard four designs
//! hand-waved. These types are therefore only ever used from non-runtime
//! threads: the outbound `FileSystem` calls run on ordinary OS / `spawn_blocking`
//! threads, and the inbound server runs inside `tokio::task::spawn_blocking`
//! (the blocking pool, not a worker). Each holds a [`Handle`] explicitly rather
//! than calling `Handle::current`, so the bridge works with no runtime entered
//! on the calling thread.

use std::io::{self, Read, Write};
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};
use tokio::runtime::Handle;

/// Runs `future` on `handle`, applying `deadline` when one is set.
///
/// The timeout is constructed *inside* the future driven by `block_on`, so the
/// runtime's time driver is already entered when the `Timeout` is created;
/// building it outside the runtime context panics with "no reactor running".
fn block_on_deadline<F, T>(handle: &Handle, deadline: Option<Duration>, future: F) -> io::Result<T>
where
    F: std::future::Future<Output = io::Result<T>>,
{
    handle.block_on(async move {
        match deadline {
            Some(deadline) => tokio::time::timeout(deadline, future)
                .await
                .map_err(|_| timed_out())?,
            None => future.await,
        }
    })
}

/// The blocking read half of an iroh QUIC bidi stream.
pub struct BlockingReader {
    recv: RecvStream,
    handle: Handle,
    deadline: Option<Duration>,
}

impl BlockingReader {
    /// Wraps the receive half, driving it on `handle` with an optional deadline.
    #[must_use]
    pub fn new(recv: RecvStream, handle: Handle, deadline: Option<Duration>) -> Self {
        Self {
            recv,
            handle,
            deadline,
        }
    }
}

impl Read for BlockingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let recv = &mut self.recv;
        let read = async move {
            recv.read(buf)
                .await
                // `Ok(None)` is a clean stream EOF, reported as a 0-byte read.
                .map(|read| read.unwrap_or(0))
                .map_err(|err| io::Error::other(format!("quic recv read failed: {err}")))
        };
        block_on_deadline(&self.handle, self.deadline, read)
    }
}

/// The blocking write half of an iroh QUIC bidi stream.
pub struct BlockingWriter {
    send: SendStream,
    handle: Handle,
    deadline: Option<Duration>,
}

impl BlockingWriter {
    /// Wraps the send half, driving it on `handle` with an optional deadline.
    #[must_use]
    pub fn new(send: SendStream, handle: Handle, deadline: Option<Duration>) -> Self {
        Self {
            send,
            handle,
            deadline,
        }
    }
}

impl Write for BlockingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let send = &mut self.send;
        let write = async move {
            send.write_all(buf)
                .await
                .map(|()| buf.len())
                .map_err(|err| io::Error::other(format!("quic send write failed: {err}")))
        };
        block_on_deadline(&self.handle, self.deadline, write)
    }

    fn flush(&mut self) -> io::Result<()> {
        // iroh streams flush implicitly on write; the stream is finished when the
        // send half drops at session end.
        Ok(())
    }
}

/// A blocking, bidirectional byte stream over one iroh QUIC bidi stream.
///
/// Satisfies [`wanix_9p_client::Duplex`] (it is [`Read`] + [`Write`] + [`Send`]),
/// so it can back a [`wanix_9p_client::RemoteFs`] without it seeing async. The
/// outbound dialer writes the 9P `Tversion` immediately, which is what makes the
/// peer's `accept_bi` resolve (an iroh bidi stream is invisible to the acceptor
/// until the opener writes its first byte).
pub struct BlockingDuplex {
    reader: BlockingReader,
    writer: BlockingWriter,
}

impl BlockingDuplex {
    /// Wraps both halves of a bidi stream, driving them on `handle`.
    #[must_use]
    pub fn new(
        send: SendStream,
        recv: RecvStream,
        handle: Handle,
        deadline: Option<Duration>,
    ) -> Self {
        Self {
            reader: BlockingReader::new(recv, handle.clone(), deadline),
            writer: BlockingWriter::new(send, handle, deadline),
        }
    }
}

impl Read for BlockingDuplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl Write for BlockingDuplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Builds an explicit timed-out I/O error for an exceeded per-op deadline.
fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "mesh stream operation exceeded its per-op deadline",
    )
}
