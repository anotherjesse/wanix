//! [`AppFsService`]: the shared state behind every principal-scoped view.
//!
//! Construction performs the v0.2 wire handshake: the guest's first line must
//! be `{"hello":{"proto":1,...}}` (see [`crate::AppHello`]) — a proto
//! mismatch or a non-hello first line is a construction-time error, and a
//! hello that declares `files`/`streams` is the tree authority over the
//! host-declared (manifest) tree. After the handshake the service owns the
//! send half of the guest channel, the operation-id counter, the reply slot,
//! and the stream table. Its central invariant is the actor property: **one
//! request in flight at a time** — `transact` holds the sender lock from send
//! until the pump (see [`crate::pump`]) has routed the reply, so the guest
//! never sees concurrent events. Every wait is bounded by the per-request
//! deadline; expiry latches the channel down (the guest is unresponsive). The
//! receive direction belongs entirely to the pump thread, which drains guest
//! output whether or not an operation is in flight. No lock here is held
//! while calling into another filesystem; fan-out touches only the adapter's
//! own stream buffers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wanix_fs::{FsError, FsResult};

use crate::channel::{AppReceiver, AppSender};
use crate::fs::AppFs;
use crate::protocol::{AppHello, AppOk, AppOp, AppRequest, GuestLine, PROTO_VERSION};
use crate::pump::{self, ReplySlot};
use crate::streams::StreamTable;
use crate::tree::AppTree;

/// Default per-request deadline: generous, but bounded — a guest that has not
/// answered one discrete op in this long is treated as unresponsive and the
/// channel latches down.
pub const DEFAULT_OP_DEADLINE: Duration = Duration::from_secs(30);

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
    op_deadline: Duration,
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
    /// op to be delivered. `range` carries the byte range for `read`.
    pub(crate) fn transact(
        &self,
        op: AppOp,
        path: &str,
        principal: &str,
        data: Option<String>,
        range: Option<(u64, u64)>,
    ) -> FsResult<AppOk> {
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = AppRequest {
            id,
            op,
            path: path.to_owned(),
            principal: principal.to_owned(),
            at_ms: now_ms(),
            data,
            offset: range.map(|(offset, _)| offset),
            len: range.map(|(_, len)| len),
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
        let outcome = self.slot.wait(op, self.op_deadline);
        // A deadline expiry latches down inside `wait`, where the pump's own
        // teardown does not run — close the stream surface here so blocked
        // stream readers observe EOF instead of waiting on a dead guest.
        if outcome.is_err() && self.slot.is_down() {
            self.streams.close_all();
        }
        outcome
    }

    /// Registers one stream subscription and returns its id and buffer.
    pub(crate) fn subscribe(
        &self,
        stream: &str,
        principal: &str,
    ) -> FsResult<(u64, Arc<wanix_fs::LineBuffer>)> {
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
}

/// Reads and validates the guest's opening hello line, bounded by the op
/// deadline: a guest that starts but never writes its hello is unresponsive
/// (the same "an unresponsive guest is a dead guest" rule as every later op),
/// and an unbounded wait here would park service construction — and the
/// restart supervisor calling it — forever.
fn read_hello(receiver: &mut dyn AppReceiver, deadline: Duration) -> FsResult<AppHello> {
    let raw = receiver
        .recv_line_deadline(deadline)
        .map_err(|err| channel_down(&err))?;
    match GuestLine::parse(&raw)? {
        GuestLine::Hello(hello) if hello.proto == PROTO_VERSION => Ok(hello),
        GuestLine::Hello(hello) => Err(FsError::Other(format!(
            "app guest speaks wire proto {}; this host speaks {PROTO_VERSION}",
            hello.proto
        ))),
        GuestLine::Reply(_) | GuestLine::Publish(_) => Err(FsError::Other(
            "app guest did not start with a hello line \
             (the v0.2 wire requires {\"hello\":{\"proto\":1}} first)"
                .to_owned(),
        )),
    }
}

/// Resolves the tree authority: the hello's declaration when present,
/// otherwise the host-declared (manifest) tree.
fn tree_from_hello(hello: &AppHello, declared: AppTree) -> FsResult<AppTree> {
    if hello.files.is_none() && hello.streams.is_none() {
        return Ok(declared);
    }
    let files: Vec<&str> = hello.files.iter().flatten().map(String::as_str).collect();
    let streams: Vec<&str> = hello.streams.iter().flatten().map(String::as_str).collect();
    AppTree::declare(&files, &streams)
        .map_err(|message| FsError::Other(format!("app guest hello: {message}")))
}

/// The file2chan adapter service: one guest channel plus host-owned streams.
///
/// Construct it once per app, then bind each verified caller through
/// [`AppFsService::open_view`] — the ToolFS principal pattern. Clone-cheap:
/// every clone shares one channel and one stream table. Construction performs
/// the blocking hello handshake on the calling thread, then spawns the pump
/// thread that owns the receive half; the service keeps the send half, so
/// dropping every service clone closes a pipe-backed guest's stdin.
#[derive(Clone)]
pub struct AppFsService {
    shared: Arc<Shared>,
}

impl AppFsService {
    /// Creates the adapter with the default per-request deadline
    /// ([`DEFAULT_OP_DEADLINE`]); see [`AppFsService::with_op_deadline`].
    ///
    /// # Errors
    ///
    /// Returns the handshake error: a broken channel, a first line that is
    /// not a valid hello, a proto mismatch, or an invalid hello tree.
    pub fn new(
        declared: AppTree,
        sender: Box<dyn AppSender>,
        receiver: Box<dyn AppReceiver>,
    ) -> FsResult<Self> {
        Self::with_op_deadline(declared, sender, receiver, DEFAULT_OP_DEADLINE)
    }

    /// Creates the adapter over a host-declared tree and the two guest
    /// channel halves, performing the blocking hello handshake (the guest's
    /// declaration wins over `declared`) and spawning the host pump that owns
    /// `receiver`. Every discrete op — and the hello handshake itself —
    /// waits at most `op_deadline` for its reply; expiry latches the channel
    /// down (for the hello: fails construction).
    ///
    /// # Errors
    ///
    /// Returns the handshake error: a broken channel, a hello that never
    /// arrived within `op_deadline`, a first line that is not a valid hello,
    /// a proto mismatch, or an invalid hello tree.
    pub fn with_op_deadline(
        declared: AppTree,
        sender: Box<dyn AppSender>,
        mut receiver: Box<dyn AppReceiver>,
        op_deadline: Duration,
    ) -> FsResult<Self> {
        let hello = read_hello(receiver.as_mut(), op_deadline)?;
        let tree = tree_from_hello(&hello, declared)?;
        let slot = Arc::new(ReplySlot::default());
        let streams = StreamTable::default();
        pump::spawn(receiver, tree.clone(), Arc::clone(&slot), streams.clone());
        Ok(Self {
            shared: Arc::new(Shared {
                tree,
                sender: Mutex::new(sender),
                next_request_id: AtomicU64::new(1),
                slot,
                streams,
                op_deadline,
            }),
        })
    }

    /// Whether the guest channel has latched down — an op deadline expired
    /// ("an unresponsive guest is a dead guest") or the channel broke. Once
    /// down, this adapter can never serve again, even if the guest task is
    /// still running (a guest wedged in an infinite loop never exits); a
    /// restart supervisor must treat latch-down as guest death.
    #[must_use]
    pub fn is_down(&self) -> bool {
        self.shared.slot.is_down()
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
    /// transport/attach layer (e.g. `iroh:<hex>` from the verified mesh peer
    /// id), never from a client payload. The view stamps it into every guest
    /// event and into the stream-subscriber registry that backs `who`.
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
