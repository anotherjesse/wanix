//! Per-topic gossip state for [`super::GossipPlumbPort`].
//!
//! A `#plumb` topic maps to one iroh-gossip [`TopicId`]. The first time a topic
//! is touched (a `send` or `recv` open), [`TopicActor::join`] subscribes to that
//! gossip topic on the node's runtime, holds the resulting [`GossipSender`] for
//! broadcasts, and spawns a pump task that forwards every [`Event::Received`]
//! into a local fan-out [`LocalPlumbPort`] so all same-node `recv` subscribers
//! observe it. The actor handle is kept alive in the port's topic map so gossip
//! membership persists for the life of the device.

use std::sync::Arc;

use bytes::Bytes;
use iroh::EndpointId;
use iroh_gossip::api::{Event, GossipSender};
use iroh_gossip::net::Gossip;
use iroh_gossip::proto::TopicId;
use n0_future::StreamExt;
use n0_future::task::AbortOnDropHandle;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use wanix_fs::{FsError, FsResult};
use wanix_plumb::{LocalPlumbPort, PlumbPort, PlumbStream};

/// One topic's live gossip subscription: a broadcast sender, a local fan-out
/// port, and the pump task forwarding received messages into the fan-out.
pub(super) struct TopicActor {
    /// The gossip broadcast sender, behind an async mutex because `broadcast` is
    /// `&self` async and we drive it from blocking `publish` via `block_on`.
    sender: Arc<Mutex<GossipSender>>,
    /// Same-node fan-out: received gossip messages are pushed here, and every
    /// local `recv` file subscribes here. This reuses the in-process port so the
    /// gossip and single-node delivery paths are identical.
    fanout: Arc<LocalPlumbPort>,
    /// The forwarding task, aborted when the actor (and so the topic) is dropped.
    _pump: AbortOnDropHandle<()>,
    handle: Handle,
}

impl TopicActor {
    /// Joins the gossip topic for `topic`, bootstrapping from `bootstrap`.
    ///
    /// Subscribes on `handle`, holds the broadcast sender, and spawns the pump
    /// that forwards received messages into a fresh local fan-out port. Does not
    /// wait for neighbors: delivery is best-effort, and a topic with no peers yet
    /// still serves same-node subscribers immediately.
    pub(super) fn join(
        gossip: &Gossip,
        topic: &str,
        bootstrap: &[EndpointId],
        handle: Handle,
    ) -> FsResult<Self> {
        let topic_id = topic_id_for(topic);
        let bootstrap = bootstrap.to_vec();
        let gossip = gossip.clone();
        let topic_handle = handle.clone();
        let gossip_topic = handle
            .block_on(async move { gossip.subscribe(topic_id, bootstrap).await })
            .map_err(|err| FsError::Other(format!("plumb gossip subscribe failed: {err}")))?;
        let (sender, receiver) = gossip_topic.split();
        let fanout = Arc::new(LocalPlumbPort::new());
        let pump_fanout = Arc::clone(&fanout);
        let topic_name = topic.to_owned();
        let pump = AbortOnDropHandle::new(topic_handle.spawn(async move {
            pump_received(receiver, pump_fanout, &topic_name).await;
        }));
        Ok(Self {
            sender: Arc::new(Mutex::new(sender)),
            fanout,
            _pump: pump,
            handle,
        })
    }

    /// Broadcasts `line` to the gossip topic and the same-node fan-out.
    ///
    /// The local fan-out push is what lets two subscribers on one node talk
    /// without a relay (gossip does not loop a node's own broadcast back to it);
    /// the gossip broadcast carries it to remote nodes. Both are best-effort.
    pub(super) fn publish(&self, topic: &str, line: &[u8]) -> FsResult<()> {
        // Same-node delivery first: never depends on gossip membership.
        self.fanout.publish(topic, line)?;
        let sender = Arc::clone(&self.sender);
        let message = Bytes::copy_from_slice(line);
        self.handle
            .block_on(async move { sender.lock().await.broadcast(message).await })
            .map_err(|err| FsError::Other(format!("plumb gossip broadcast failed: {err}")))
    }

    /// Subscribes a new local `recv` stream to this topic's fan-out.
    pub(super) fn subscribe(&self, topic: &str) -> FsResult<Box<dyn PlumbStream>> {
        self.fanout.subscribe(topic)
    }

    /// Whether this topic still has any live `recv` subscriber on this node.
    ///
    /// Once every `recv` stream the fan-out produced has dropped, the topic is
    /// idle: the port may evict the actor, dropping its gossip membership and
    /// aborting the pump task, so a caller churning through topic names cannot
    /// accumulate memberships and tasks without bound.
    pub(super) fn has_live_subscribers(&self) -> bool {
        self.fanout.has_live_subscribers()
    }
}

/// Forwards every received gossip message into the local fan-out port.
///
/// Runs until the receiver stream ends (the topic is dropped). Non-`Received`
/// events (neighbor up/down, lag) are ignored: the bus only carries message
/// bytes, and a lagged subscriber simply misses messages, matching best-effort.
async fn pump_received(
    mut receiver: iroh_gossip::api::GossipReceiver,
    fanout: Arc<LocalPlumbPort>,
    topic: &str,
) {
    while let Some(event) = receiver.next().await {
        if let Ok(Event::Received(message)) = event {
            // Push the raw received line into the fan-out; same-node subscribers
            // drain it through their `recv` streams. A push to a topic with no
            // local subscribers is a best-effort drop.
            let _ = fanout.publish(topic, &message.content);
        }
    }
}

/// Maps a topic name to its gossip [`TopicId`] via BLAKE3, per the blueprint.
///
/// `TopicId::from_bytes(blake3(topic))` gives a stable 32-byte topic id from any
/// UTF-8 name, so two nodes naming the same topic join the same gossip swarm
/// without coordinating ids out of band.
pub(super) fn topic_id_for(topic: &str) -> TopicId {
    TopicId::from_bytes(*blake3::hash(topic.as_bytes()).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::topic_id_for;

    #[test]
    fn topic_id_is_stable_and_name_keyed() {
        // The same name always maps to the same gossip topic id (so two nodes
        // join the same swarm), and different names map to different ids.
        assert_eq!(topic_id_for("build"), topic_id_for("build"));
        assert_ne!(topic_id_for("build"), topic_id_for("deploy"));
    }

    #[test]
    fn topic_id_is_blake3_of_the_name() {
        let expected = *blake3::hash(b"build").as_bytes();
        assert_eq!(topic_id_for("build").as_bytes(), &expected);
    }
}
