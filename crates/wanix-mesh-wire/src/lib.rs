//! Native FileSystem-over-stream wire for the Wanix mesh.
//!
//! This crate is the typed wire *contract and codec* for Wanix↔Wanix mesh
//! traffic: a hand-rolled, length-prefixed `postcard` frame over the existing
//! sync `Duplex: Read + Write + Send` byte-stream boundary. It is the native
//! analog of `wanix-9p` (server codec) + `wanix-9p-client` (client codec), and
//! like them it is **transport-agnostic and async-free** — it has no knowledge
//! of iroh, tokio, or QUIC. `wanix-mesh` (the single async/iroh edge) composes
//! this codec onto its iroh endpoint, wrapping each QUIC bidi stream in the
//! existing `BlockingDuplex` so the wire crate only ever sees synchronous
//! `Read`/`Write`.
//!
//! The design rationale (hand-rolled frame over `irpc`, the per-stream/typed-
//! error/per-principal wins, and the op-by-op mapping) lives in
//! `docs/design/native-mesh-wire.md`; the contract it serves lives in ADR 0004.
//!
//! # Phase 1 surface
//!
//! This crate currently ships the value/error mirrors and the framing layer:
//!
//! - [`WireFsError`] — the typed [`wanix_fs::FsError`] mirror that crosses the
//!   wire losslessly (no errno round-trip).
//! - [`WireMetadata`], [`WireDirEntry`], [`WireFileType`], [`WireOpenOptions`],
//!   [`WireSeek`] — `postcard`-serializable mirrors of the `wanix-fs` value
//!   types, with lossless `From`/`Into` converters.
//! - [`write_frame`] / [`read_frame`] and the [`MAX_FRAME_LEN`] /
//!   [`MAX_CHUNK_LEN`] ceilings — bounded `postcard` framing over a sync stream.
//!
//! # Phase 2 surface
//!
//! On top of Phase 1 this crate now ships the protocol enums, the sync server
//! dispatcher, and the sync client `FileSystem` facade — fully exercisable over
//! an in-memory [`Duplex`] with no transport:
//!
//! - [`Inbound`] / [`FsRequest`] / [`FsResponse`] / [`ReadDirPage`] /
//!   [`OpenRequest`] / [`OpenResponse`] / [`OpenOk`] / [`FileOp`] /
//!   [`FileReply`] — the op-level service protocol.
//! - [`serve_one`] — the sync server dispatcher: one [`Inbound`] in, one
//!   [`FsResponse`] out (one-shot), or the open-file loop (open).
//! - [`StreamFactory`], [`NativeFs`], [`NativeFile`] — the sync client facade
//!   over a [`Duplex`]-producing factory.
//!
//! Binding this codec to iroh QUIC (the async/iroh edge) lands in `wanix-mesh`.

mod client;
mod error;
mod file;
mod frame;
mod proto;
mod server;
mod value;

use std::io::{Read, Write};

pub use client::{NativeFs, StreamFactory};
pub use error::WireFsError;
pub use file::NativeFile;
pub use frame::{FrameError, FrameResult, MAX_CHUNK_LEN, MAX_FRAME_LEN, read_frame, write_frame};
pub use proto::{
    FileOp, FileReply, FsRequest, FsResponse, Inbound, OpenOk, OpenRequest, OpenResponse,
    ReadDirPage,
};
pub use server::serve_one;
pub use value::{WireDirEntry, WireFileType, WireMetadata, WireOpenOptions, WireSeek};

/// The content-addressing primitive carried by `content_hash`, re-exported from
/// `wanix-fs` so callers need not depend on it directly.
pub use wanix_fs::ContentHash;

/// A blocking, bidirectional byte stream the native wire runs over.
///
/// This is the transport seam: the wire crate frames `postcard` requests and
/// replies over any type that is [`Read`] + [`Write`] + [`Send`], exactly the
/// boundary `wanix-9p-client`'s `Duplex` defines. `wanix-mesh` supplies the
/// implementation by wrapping an iroh QUIC bidi stream in its existing
/// `BlockingDuplex`; the wire crate never sees async. Any in-memory pipe
/// satisfies it for tests.
pub trait Duplex: Read + Write + Send {}

impl<T: Read + Write + Send> Duplex for T {}
