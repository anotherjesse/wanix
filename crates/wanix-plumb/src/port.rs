//! [`PlumbPort`]: the pluggable backend the `#plumb` device publishes to and
//! subscribes from.
//!
//! The `#plumb` device is a pure synchronous [`wanix_fs::FileSystem`]; the
//! transport that actually carries a topic's messages between nodes is injected
//! as a `PlumbPort`. A [`crate::LocalPlumbPort`] delivers in-process (for tests
//! and single-node use); `wanix-mesh`'s `GossipPlumbPort` maps each topic to an
//! iroh-gossip `TopicId` and bridges `publish` to a gossip broadcast and a
//! subscription to the topic's received events. Keeping this seam sync and
//! iroh-free is what lets the device live in a core crate.

use std::sync::Arc;

use wanix_fs::FsResult;

/// A backend that carries `#plumb` topic messages.
///
/// Delivery is best-effort: [`Self::publish`] hands a serialized envelope to the
/// transport with no acknowledgement, and a subscription started after a
/// publish never sees that earlier message. Implementations must be cheap to
/// `subscribe` repeatedly (one subscription per open `recv` file).
pub trait PlumbPort: Send + Sync {
    /// Publishes one already-serialized envelope line to `topic`.
    ///
    /// `line` is a newline-terminated JSON envelope (see
    /// [`crate::PlumbEnvelope::to_line`]). Delivery is best-effort; a publish to
    /// a topic with no current subscribers is silently dropped on the wire.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the transport rejects the message (for
    /// example, an oversized payload or a closed backend).
    fn publish(&self, topic: &str, line: &[u8]) -> FsResult<()>;

    /// Subscribes to `topic`, returning a blocking byte stream of received
    /// envelope lines.
    ///
    /// Each call returns an independent subscription, so opening `recv` twice on
    /// one topic yields two streams that each see messages broadcast after they
    /// subscribed. The returned stream blocks until a message arrives or the
    /// subscription is dropped.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the subscription cannot be created.
    fn subscribe(&self, topic: &str) -> FsResult<Box<dyn PlumbStream>>;
}

/// A live subscription's blocking byte stream of received envelope lines.
///
/// The bytes are a concatenation of newline-terminated JSON envelopes, in
/// arrival order. [`Self::read`] blocks until at least one byte is available;
/// it returns `Ok(0)` only when the subscription has permanently closed (the
/// backend shut down), matching the pipe/agent stream end-of-stream contract.
pub trait PlumbStream: Send {
    /// Drains received envelope bytes into `buf`, blocking until data arrives or
    /// the subscription closes (`Ok(0)`).
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the subscription's buffer is poisoned.
    fn read(&mut self, buf: &mut [u8]) -> FsResult<usize>;

    /// Whether a [`Self::read`] would return without blocking (data buffered or
    /// the subscription closed).
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when readiness cannot be determined.
    fn read_ready(&self) -> FsResult<bool>;
}

/// A shared, reference-counted [`PlumbPort`] the device holds.
pub type SharedPlumbPort = Arc<dyn PlumbPort>;
