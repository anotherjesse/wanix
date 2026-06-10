//! AppFS v0 — the file2chan adapter: a guest-defined `FileSystem`.
//!
//! This crate is the filesystem surface of an AppResource (`docs/appfs.md`):
//! a mounted [`wanix_fs::FileSystem`] whose *discrete* operations — `read`,
//! `write`, `readdir`, `stat` on declared guest paths — become newline-JSON
//! events delivered to a guest app over a byte channel, in the lineage of
//! Inferno's `file2chan` (filesystem requests arrive as messages to a
//! process). The guest decides; the host moves bytes:
//!
//! - **Discrete ops go to the guest, one at a time.** The adapter serializes
//!   requests over the [`AppSender`] under one actor lock — the guest is a
//!   single actor and never sees concurrent events. The guest→host direction
//!   is owned by a dedicated pump thread (the [`AppReceiver`]'s single
//!   reader), so guest output is always drained: publishes deliver on
//!   arrival, a bounded stdio pipe cannot deadlock against a large request,
//!   and a guest protocol violation latches the channel down
//!   ([`wanix_fs::FsError::Unreachable`] thereafter) instead of
//!   desynchronizing the reply stream.
//! - **Stream files never touch the guest.** Paths declared as streams in the
//!   [`AppTree`] are host-owned: each open registers a bounded, lossy,
//!   never-EOF subscription buffer (the `#plumb` `LineBuffer` discipline:
//!   1 MiB ceiling, drop-oldest, condvar-blocking reads). A wedged guest
//!   cannot block an existing stream reader, and a slow reader loses the
//!   oldest backlog instead of blocking the app. The guest feeds streams by
//!   emitting `{"publish":{...}}` lines, which the host fans out to every
//!   subscriber of that stream. Never-EOF holds *while the guest lives*:
//!   when the guest app exits, the host tears the stream surface down through
//!   [`AppStreamCloser`] so blocked readers observe EOF instead of parking
//!   forever on an app that can never publish again.
//! - **Presence is host session state.** The reserved `who` file lists the
//!   principals currently holding open stream subscriptions, one per line;
//!   the guest is not consulted.
//! - **Identity comes from the transport, never the payload.**
//!   [`AppFsService::open_view`] binds an opaque principal string into an
//!   [`AppFs`] view (the ToolFS pattern); the view stamps that principal into
//!   every guest event. The `FileSystem` itself stays principal-blind.
//!
//! What v0 is **not** (see `docs/appfs.md` §Non-Goals): no HTTP surface, no
//! turn scheduler or task lifecycle (the channel is a trait seam — the CLI
//! wires it to a resident guest task later), and no `StreamHandle` value
//! algebra — streams here are declared paths, not first-class returned
//! capabilities. The wire protocol is pinned in [`protocol`]'s types and
//! tests; its error vocabulary mirrors [`wanix_fs::FsError`] (it is *not* the
//! ADR 0009 job taxonomy).

/// Short human-readable crate responsibility used by workspace smoke tests.
pub const CRATE_PURPOSE: &str = "wanix app filesystem adapter (file2chan)";

mod buffer;
mod channel;
mod files;
mod fs;
mod protocol;
mod pump;
mod service;
mod streams;
mod tree;

pub use buffer::LineBuffer;
pub use channel::{AppReceiver, AppSender};
pub use fs::AppFs;
pub use protocol::{
    AppDirEntry, AppErr, AppErrKind, AppOk, AppOp, AppPublish, AppReply, AppRequest, GuestLine,
    MAX_LINE_LEN, decode_data, encode_data, publish_to_line,
};
pub use service::{AppFsService, AppStreamCloser};
pub use tree::{AppTree, WHO_FILE};

#[cfg(test)]
mod tests;
