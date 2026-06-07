//! [`GossipPlumbPort`]: the iroh-gossip backend for the `#plumb` bus.
//!
//! This is the mesh edge of the plumber. The synchronous `#plumb` device
//! ([`wanix_plumb::PlumbDevice`]) is reused **unchanged**; this module only
//! implements its [`wanix_plumb::PlumbPort`] backend over iroh-gossip. Each
//! topic name maps to a gossip [`TopicId`](iroh_gossip::proto::TopicId) via
//! `TopicId::from_bytes(blake3(topic))` (see [`topic::topic_id_for`]), `send`
//! broadcasts on that topic, and `recv` drains the messages the topic receives
//! — so a `task.done` posted to `#plumb/build` on one node is read from
//! `#plumb/build/recv` on another. This is the plumber, made real on the mesh:
//! typed, best-effort, broker-less coordination between agents and tools.
//!
//! # Layering and trust
//!
//! The gossip ALPN rides the same identity-bound endpoint as the 9P control
//! plane and the blob data plane ([`Gossip::builder().spawn`](iroh_gossip::net::Gossip)),
//! so a peer reaches all three over one QUIC path. Gossip carries only message
//! bytes — it grants no filesystem or exec capability — so unlike `#task`/`#agent`
//! it is safe on the public endpoint; the trust boundary that matters is the
//! topic name (anyone who knows it can read and write the bus).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use iroh::address_lookup::memory::MemoryLookup;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use iroh_gossip::net::Gossip;
use tokio::runtime::Handle;
use wanix_fs::{FsError, FsResult};
use wanix_plumb::{PlumbPort, PlumbStream};

mod topic;

use topic::TopicActor;

/// Re-export of the iroh-gossip ALPN, the coordination-plane wire contract
/// registered on the shared [`MeshNode`](crate::MeshNode) endpoint.
pub use iroh_gossip::ALPN as GOSSIP_ALPN;

/// Hard ceiling on the number of live per-topic actors a single port retains.
///
/// Each actor is one gossip swarm membership plus one spawned pump task, and a
/// topic name is caller-controlled — over an imported `#plumb`, a remote 9P
/// client supplies it through walk paths. Without a cap, repeated
/// `#plumb/<distinct-name>/{send,recv}` opens would accumulate memberships and
/// tasks without bound. Idle actors (no live `recv` subscriber) are swept first;
/// this ceiling is the backstop when many topics have live subscribers at once.
const MAX_LIVE_TOPICS: usize = 1024;

/// The iroh-gossip [`PlumbPort`] backing a mesh `#plumb` device.
///
/// Holds one [`Gossip`] instance over the node's endpoint and a lazily populated
/// map of per-topic actors. The first `send`/`recv` on a topic joins its gossip
/// swarm and starts forwarding received messages. An actor is kept alive while
/// the topic has a live `recv` subscriber; once every subscriber drops it
/// becomes idle and is swept on the next topic lookup, releasing its gossip
/// membership and aborting its pump task so a caller churning through topic
/// names cannot accumulate memberships and tasks without bound
/// ([`MAX_LIVE_TOPICS`] caps the live set as a backstop).
#[derive(Clone)]
pub struct GossipPlumbPort {
    gossip: Gossip,
    handle: Handle,
    bootstrap: Arc<Vec<EndpointId>>,
    topics: Arc<Mutex<BTreeMap<String, Arc<TopicActor>>>>,
}

impl GossipPlumbPort {
    /// Builds a gossip plumb port over `endpoint`, driving gossip on `handle`.
    ///
    /// `bootstrap` are dialable peer addresses ([`EndpointAddr`] tickets) a
    /// freshly joined topic dials to enter the swarm. Their direct addresses are
    /// registered with the endpoint's address lookup so a relay-less node can
    /// reach them by id; the peer ids become the gossip bootstrap set. An empty
    /// list still serves same-node subscribers and accepts inbound gossip once
    /// the ALPN is registered on the node's
    /// [`Router`](iroh::protocol::Router) via [`crate::MeshNode::serve_with_plumb`].
    #[must_use]
    pub fn new(endpoint: Endpoint, handle: Handle, bootstrap: Vec<EndpointAddr>) -> Self {
        // Teach the endpoint how to reach each bootstrap peer by id (direct
        // addresses), so a relay-disabled node can dial the gossip swarm. iroh's
        // own gossip tests use this `MemoryLookup` seam for exactly this.
        if let Ok(lookup) = endpoint.address_lookup() {
            let memory = MemoryLookup::with_provenance("wanix-plumb-bootstrap");
            for addr in &bootstrap {
                memory.add_endpoint_info(addr.clone());
            }
            lookup.add(memory);
        }
        let peers: Vec<EndpointId> = bootstrap.iter().map(|addr| addr.id).collect();
        let gossip = handle.block_on(async { Gossip::builder().spawn(endpoint) });
        Self {
            gossip,
            handle,
            bootstrap: Arc::new(peers),
            topics: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Returns the underlying [`Gossip`] protocol handler for
    /// [`Router`](iroh::protocol::Router) registration on the node's endpoint
    /// under [`GOSSIP_ALPN`].
    #[must_use]
    pub fn protocol(&self) -> Gossip {
        self.gossip.clone()
    }

    /// Number of per-topic actors currently retained, for eviction tests.
    ///
    /// Exposes the live-topic count so an e2e test can prove that churning
    /// through caller-controlled topic names does not accumulate gossip
    /// memberships and pump tasks without bound.
    #[doc(hidden)]
    #[must_use]
    pub fn live_topic_count(&self) -> usize {
        self.topics.lock().map(|t| t.len()).unwrap_or(0)
    }

    /// Returns or lazily joins the actor for `topic`.
    ///
    /// Sweeps idle actors (no live `recv` subscriber) before any insert, so a
    /// topic touched once — a one-shot `send`, or a `recv` whose stream has since
    /// dropped — does not pin a gossip membership and pump task. The existing
    /// topic is never swept, so an in-flight publish to a live topic is safe.
    /// Refuses to grow past [`MAX_LIVE_TOPICS`] live topics as a backstop.
    fn actor(&self, topic: &str) -> FsResult<Arc<TopicActor>> {
        let mut topics = self
            .topics
            .lock()
            .map_err(|_| FsError::Other("plumb gossip topic map poisoned".to_owned()))?;
        if let Some(actor) = topics.get(topic) {
            return Ok(Arc::clone(actor));
        }
        // Reclaim every idle topic before adding a new one: dropping the actor
        // releases its gossip membership and aborts its pump task. This bounds
        // accumulation across churned, caller-controlled topic names.
        topics.retain(|_, actor| actor.has_live_subscribers());
        if topics.len() >= MAX_LIVE_TOPICS {
            return Err(FsError::Other(format!(
                "plumb gossip topic limit of {MAX_LIVE_TOPICS} live topics reached"
            )));
        }
        let actor = Arc::new(TopicActor::join(
            &self.gossip,
            topic,
            &self.bootstrap,
            self.handle.clone(),
        )?);
        topics.insert(topic.to_owned(), Arc::clone(&actor));
        Ok(actor)
    }
}

impl PlumbPort for GossipPlumbPort {
    fn publish(&self, topic: &str, line: &[u8]) -> FsResult<()> {
        self.actor(topic)?.publish(topic, line)
    }

    fn subscribe(&self, topic: &str) -> FsResult<Box<dyn PlumbStream>> {
        self.actor(topic)?.subscribe(topic)
    }
}

#[cfg(test)]
mod tests {
    use super::GOSSIP_ALPN;

    #[test]
    fn gossip_alpn_is_the_iroh_gossip_contract() {
        // The ALPN is iroh-gossip's own constant; the device does not invent one.
        assert!(!GOSSIP_ALPN.is_empty());
    }
}
