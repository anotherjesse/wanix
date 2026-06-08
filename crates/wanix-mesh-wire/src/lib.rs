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
//! The protocol enums, the sync server dispatcher, and the sync client
//! `FileSystem` facade land in later phases.

mod error;
mod frame;
mod value;

pub use error::WireFsError;
pub use frame::{FrameError, FrameResult, MAX_CHUNK_LEN, MAX_FRAME_LEN, read_frame, write_frame};
pub use value::{WireDirEntry, WireFileType, WireMetadata, WireOpenOptions, WireSeek};
