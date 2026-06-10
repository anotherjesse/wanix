//! [`AppFsService`]: the shared state behind every principal-scoped view.
//!
//! The service owns the send half of the guest channel, the operation-id
//! counter, the reply slot, and the stream table. Its central invariant is
//! the actor property: **one request in flight at a time** — `transact`
//! holds the sender lock from send until the pump (see [`crate::pump`]) has
//! routed the reply, so the guest never sees concurrent events. The receive
//! direction belongs entirely to the pump thread, which drains guest output
//! whether or not an operation is in flight. No lock here is held while
//! calling into another filesystem; fan-out touches only the adapter's own
//! stream buffers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use wanix_fs::{FsError, FsResult};

use crate::channel::{AppReceiver, AppSender};
use crate::fs::AppFs;
use crate::protocol::{AppOk, AppOp, AppRequest};
use crate::pump::{self, ReplySlot};
use crate::streams::StreamTable;
use crate::tree::AppTree;

pub(crate) fn channel_down(err: &std::io::Error) -> FsError {
    FsError::Unreachable(format!("app channel: {err}"))
}

/// The shared adapter state every [`AppFs`] view and open file points at.
pub(crate) struct Shared {
    tree: AppTree,
    sender: Mutex<Box<dyn AppSender>>,
    next_request_id: AtomicU64,
    slot: Arc<ReplySlot>,
    streams: StreamTable,
}

impl Shared {
    pub(crate) fn tree(&self) -> &AppTree {
        &self.tree
    }

    /// Sends one discrete operation to the guest and waits for its reply.
    ///
    /// The sender lock is the actor lock: held from send until the pump has
    /// routed this request's reply, so requests are serialized one at a
    /// time. Guest publishes are the pump's business and need no in-flight
    /// op to be delivered.
    pub(crate) fn transact(
        &self,
        op: AppOp,
        path: &str,
        principal: &str,
        data: Option<String>,
    ) -> FsResult<AppOk> {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = AppRequest {
            id,
            op,
            path: path.to_owned(),
            principal: principal.to_owned(),
            data,
        };
        let line = request.to_line()?;
        let mut sender = self
            .sender
            .lock()
            .map_err(|_| FsError::Other("app channel lock poisoned".to_owned()))?;
        self.slot.begin(id)?;
        if let Err(err) = sender.send_line(&line) {
            self.slot.abandon();
            return Err(channel_down(&err));
        }
        self.slot.wait()
    }

    /// Registers one stream subscription and returns its id and buffer.
    pub(crate) fn subscribe(
        &self,
        stream: &str,
        principal: &str,
    ) -> FsResult<(u64, Arc<crate::buffer::LineBuffer>)> {
        self.streams.subscribe(stream, principal)
    }

    /// Removes one stream subscription (called when its open file drops).
    pub(crate) fn unsubscribe(&self, id: u64) {
        self.streams.unsubscribe(id);
    }

    /// Renders the `who` presence snapshot (see [`StreamTable::who_snapshot`]).
    pub(crate) fn who_snapshot(&self) -> FsResult<Vec<u8>> {
        self.streams.who_snapshot()
    }
}

/// The file2chan adapter service: one guest channel plus host-owned streams.
///
/// Construct it once per app, then bind each verified caller through
/// [`AppFsService::open_view`] — the ToolFS principal pattern. Clone-cheap:
/// every clone shares one channel and one stream table. Construction spawns
/// the pump thread that owns the receive half; the service keeps the send
/// half, so dropping every service clone closes a pipe-backed guest's stdin.
#[derive(Clone)]
pub struct AppFsService {
    shared: Arc<Shared>,
}

impl AppFsService {
    /// Creates the adapter over a declared tree and the two guest channel
    /// halves, spawning the host pump that owns `receiver`.
    #[must_use]
    pub fn new(tree: AppTree, sender: Box<dyn AppSender>, receiver: Box<dyn AppReceiver>) -> Self {
        let slot = Arc::new(ReplySlot::default());
        let streams = StreamTable::default();
        pump::spawn(receiver, tree.clone(), Arc::clone(&slot), streams.clone());
        Self {
            shared: Arc::new(Shared {
                tree,
                sender: Mutex::new(sender),
                next_request_id: AtomicU64::new(1),
                slot,
                streams,
            }),
        }
    }

    /// Returns the guest-lifecycle teardown handle for this adapter's streams.
    ///
    /// The closer holds only the stream table — not the guest channel — so an
    /// exit watcher can keep one without keeping the guest's stdin pipe (and
    /// therefore the guest itself) alive. The pump closes the stream surface
    /// itself when the channel dies; this handle stays for host-side teardown
    /// that observes guest exit through other means first.
    #[must_use]
    pub fn stream_closer(&self) -> AppStreamCloser {
        AppStreamCloser {
            streams: self.shared.streams.clone(),
        }
    }

    /// Binds `principal` to a scoped [`AppFs`] view.
    ///
    /// The principal string is opaque to this crate and must come from the
    /// transport/attach layer (e.g. the verified mesh peer id), never from a
    /// client payload. The view stamps it into every guest event and into the
    /// stream-subscriber registry that backs `who`.
    #[must_use]
    pub fn open_view(&self, principal: impl Into<String>) -> AppFs {
        AppFs::new(Arc::clone(&self.shared), principal.into())
    }
}

/// Closes the host-owned stream surface when the guest app dies.
///
/// Stream files are honestly never-EOF *while the guest lives*; once the
/// guest has exited it can never publish again, so the lifecycle-honest move
/// is the opposite: every blocked stream reader is released with EOF (and
/// every later subscription starts at EOF) instead of hanging forever.
/// Discrete ops keep their own honesty through the broken channel
/// ([`wanix_fs::FsError::Unreachable`]). Obtain one via
/// [`AppFsService::stream_closer`] before handing the service out.
pub struct AppStreamCloser {
    streams: StreamTable,
}

impl AppStreamCloser {
    /// Closes every current subscription buffer and marks the stream surface
    /// permanently down. Idempotent.
    pub fn close_all(&self) {
        self.streams.close_all();
    }
}
