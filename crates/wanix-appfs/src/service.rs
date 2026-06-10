//! [`AppFsService`]: the shared state behind every principal-scoped view.
//!
//! The service owns the guest channel, the operation-id counter, and the
//! stream-subscriber registry. Its central invariant is the actor property:
//! **one request in flight at a time** — `transact` serializes send/recv
//! under the channel lock, so the guest never sees concurrent events. The
//! channel lock is never held while calling into another filesystem; the only
//! nested lock is the adapter's own subscriber registry (for publish
//! fan-out), and no path acquires the locks in the opposite order.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use wanix_fs::{FsError, FsResult};

use crate::buffer::LineBuffer;
use crate::channel::AppChannel;
use crate::fs::AppFs;
use crate::protocol::{AppOk, AppOp, AppPublish, AppRequest, GuestLine, decode_data};
use crate::tree::AppTree;

fn channel_down(err: &std::io::Error) -> FsError {
    FsError::Unreachable(format!("app channel: {err}"))
}

struct Subscriber {
    stream: String,
    principal: String,
    buffer: Arc<LineBuffer>,
}

#[derive(Default)]
struct Subscribers {
    next_id: u64,
    table: HashMap<u64, Subscriber>,
    /// Set by [`AppStreamCloser::close_all`] once the guest app has exited:
    /// every current buffer is closed (blocked readers observe EOF) and every
    /// later subscription starts closed, so no stream read can hang on an app
    /// that will never publish again.
    closed: bool,
}

/// The shared adapter state every [`AppFs`] view and open file points at.
pub(crate) struct Shared {
    tree: AppTree,
    channel: Mutex<Box<dyn AppChannel>>,
    next_request_id: AtomicU64,
    subscribers: Arc<Mutex<Subscribers>>,
}

impl Shared {
    pub(crate) fn tree(&self) -> &AppTree {
        &self.tree
    }

    /// Sends one discrete operation to the guest and waits for its reply.
    ///
    /// Guest-initiated publish lines arriving before the reply are fanned out
    /// to stream subscribers while waiting (still under the channel lock —
    /// fan-out touches only the adapter's own buffers, never another
    /// filesystem).
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
        let mut channel = self
            .channel
            .lock()
            .map_err(|_| FsError::Other("app channel lock poisoned".to_owned()))?;
        channel.send_line(&line).map_err(|err| channel_down(&err))?;
        loop {
            let raw = channel.recv_line().map_err(|err| channel_down(&err))?;
            match GuestLine::parse(&raw)? {
                GuestLine::Publish(publish) => self.fan_out(&publish)?,
                GuestLine::Reply(reply) => {
                    if reply.id != id {
                        return Err(FsError::Other(format!(
                            "app reply id {} does not match in-flight request {id}",
                            reply.id
                        )));
                    }
                    return reply.into_result();
                }
            }
        }
    }

    /// Appends one guest publish to every subscriber buffer of its stream.
    fn fan_out(&self, publish: &AppPublish) -> FsResult<()> {
        if !self.tree.is_stream(&publish.stream) {
            return Err(FsError::Other(format!(
                "app publish names undeclared stream {:?}",
                publish.stream
            )));
        }
        let bytes = decode_data(&publish.data)?;
        let subscribers = self.lock_subscribers()?;
        for subscriber in subscribers.table.values() {
            if subscriber.stream == publish.stream {
                subscriber.buffer.push(&bytes);
            }
        }
        Ok(())
    }

    fn lock_subscribers(&self) -> FsResult<std::sync::MutexGuard<'_, Subscribers>> {
        self.subscribers
            .lock()
            .map_err(|_| FsError::Other("app subscriber registry lock poisoned".to_owned()))
    }

    /// Registers one stream subscription and returns its id and buffer.
    pub(crate) fn subscribe(
        &self,
        stream: &str,
        principal: &str,
    ) -> FsResult<(u64, Arc<LineBuffer>)> {
        let mut subscribers = self.lock_subscribers()?;
        let id = subscribers.next_id;
        subscribers.next_id += 1;
        let buffer = Arc::new(LineBuffer::default());
        if subscribers.closed {
            // The guest is gone: the subscription still registers (so the
            // open succeeds and `who` stays truthful) but reads see EOF
            // immediately instead of parking on a stream that can never
            // receive another publish.
            buffer.close();
        }
        subscribers.table.insert(
            id,
            Subscriber {
                stream: stream.to_owned(),
                principal: principal.to_owned(),
                buffer: Arc::clone(&buffer),
            },
        );
        Ok((id, buffer))
    }

    /// Removes one stream subscription (called when its open file drops).
    pub(crate) fn unsubscribe(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.table.remove(&id);
        }
    }

    /// Renders the `who` presence snapshot: the unique principals currently
    /// holding open stream subscriptions, sorted, one per line.
    pub(crate) fn who_snapshot(&self) -> FsResult<Vec<u8>> {
        let subscribers = self.lock_subscribers()?;
        let principals: BTreeSet<&str> = subscribers
            .table
            .values()
            .map(|subscriber| subscriber.principal.as_str())
            .collect();
        let mut bytes = Vec::new();
        for principal in principals {
            bytes.extend_from_slice(principal.as_bytes());
            bytes.push(b'\n');
        }
        Ok(bytes)
    }
}

/// The file2chan adapter service: one guest channel plus host-owned streams.
///
/// Construct it once per app, then bind each verified caller through
/// [`AppFsService::open_view`] — the ToolFS principal pattern. Clone-cheap:
/// every clone shares one channel and one subscriber registry.
#[derive(Clone)]
pub struct AppFsService {
    shared: Arc<Shared>,
}

impl AppFsService {
    /// Creates the adapter over a declared tree and a guest channel.
    #[must_use]
    pub fn new(tree: AppTree, channel: Box<dyn AppChannel>) -> Self {
        Self {
            shared: Arc::new(Shared {
                tree,
                channel: Mutex::new(channel),
                next_request_id: AtomicU64::new(1),
                subscribers: Arc::new(Mutex::new(Subscribers::default())),
            }),
        }
    }

    /// Returns the guest-lifecycle teardown handle for this adapter's streams.
    ///
    /// The closer holds only the subscriber registry — not the guest channel —
    /// so an exit watcher can keep one without keeping the guest's stdin pipe
    /// (and therefore the guest itself) alive.
    #[must_use]
    pub fn stream_closer(&self) -> AppStreamCloser {
        AppStreamCloser {
            subscribers: Arc::clone(&self.shared.subscribers),
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
    subscribers: Arc<Mutex<Subscribers>>,
}

impl AppStreamCloser {
    /// Closes every current subscription buffer and marks the stream surface
    /// permanently down. Idempotent.
    pub fn close_all(&self) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.closed = true;
            for subscriber in subscribers.table.values() {
                subscriber.buffer.close();
            }
        }
    }
}
