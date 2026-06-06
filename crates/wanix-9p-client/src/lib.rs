//! 9P client `FileSystem` backed by Rust Wanix filesystem contracts.
//!
//! This crate is the exact mirror of `wanix-9p`'s server: where the server
//! decodes T-messages and encodes R-messages, the client encodes T-messages and
//! decodes R-messages. It is a synchronous state machine over codecs that
//! already live in `wanix-protocol`, with no transport, async runtime, or new
//! wire code of its own. [`RemoteFs`] satisfies the fully synchronous
//! [`wanix_fs::FileSystem`]/[`wanix_fs::File`] traits with no async leakage.
//!
//! The crate depends only on `wanix-fs` and `wanix-protocol`. A caller supplies
//! any blocking, bidirectional byte stream (a [`transport::Duplex`]) and
//! [`RemoteFs::connect`] negotiates the session and binds the served root.
//!
//! # Design corrections baked in
//!
//! - The framed read loop rejects any frame larger than the negotiated `msize`,
//!   poisoning the connection, so a hostile server cannot force an unbounded
//!   allocation.
//! - Files are seekable only when the server reports them as regular at open;
//!   streamed devices report `is_seekable() == false` and refuse `seek`.
//! - Fids and tags are reclaimed through RAII guards on every path, including a
//!   panic, so no fid leaks into the server's map.
//! - Directory listings are bounded and best-effort snapshots.
//! - Append is delegated to the server via `O_APPEND`, never raced client-side.

mod attr;
mod conn;
mod error;
mod fid;
mod file;
mod open;
mod readdir;
mod remote;
mod transport;
mod walk;

pub use attr::metadata_from_attr;
pub use conn::{GOOGLE_WALKGETATTR_VERSION, P9Conn, ROOT_FID};
pub use error::{ClientError, ClientResult, fs_error_for_errno};
pub use fid::{FidPool, ScratchFid, TagPool};
pub use file::RemoteFile;
pub use open::open_flags_for;
pub use remote::RemoteFs;
pub use transport::{Duplex, read_one_frame, write_frame};

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix 9P filesystem client adapters";
