//! [`MeshNode`]: one iroh endpoint per node, identity-bound and ALPN-scoped.
//!
//! A [`MeshNode`] owns a dedicated multi-thread tokio runtime, an
//! [`iroh::Endpoint`] bound from the node's persisted [`NodeIdentity`] secret
//! key, and (when serving) a [`Router`] dispatching ALPN [`crate::WANIX_9P_ALPN`]
//! to a [`P9ProtocolHandler`]. All async iroh contact is confined here; callers
//! receive only synchronous values: a [`PeerId`], an [`EndpointAddr`] ticket, and
//! a [`MeshDialer`].
//!
//! The runtime is held by the node, never entered on the caller's thread, so the
//! [`BlockingReader`]/[`BlockingWriter`]/[`BlockingDuplex`] bridges always
//! `block_on` from non-runtime threads.

use std::time::Duration;

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr};
use tokio::runtime::{Handle, Runtime};
use wanix_id::{NodeIdentity, PeerId};

use crate::dialer::MeshDialer;
use crate::error::{MeshError, MeshResult};
use crate::handler::{P9ProtocolHandler, ServeConfig};
use crate::identity::{peer_id_for, secret_key_for};

/// Default per-operation deadline on mesh streams, bounding slow-peer stalls.
pub const DEFAULT_OP_DEADLINE: Duration = Duration::from_secs(30);

/// Hard cap on concurrently served sessions (one blocking-pool thread each).
///
/// Every live inbound session pins one blocking-pool thread inside `block_on`
/// on the server's idle read for the session's lifetime. The blueprint requires
/// this be stated as a hard cap and sized, paired with the per-op deadline, to
/// close the slow-peer DoS: a flood of half-open mounts cannot exhaust the
/// process. The runtime's blocking pool is sized to admit this many sessions
/// plus headroom for the short-lived `spawn_blocking` work the runtime itself
/// schedules.
pub const MAX_CONCURRENT_SESSIONS: usize = 512;

/// Blocking-pool headroom above [`MAX_CONCURRENT_SESSIONS`] for transient work.
const BLOCKING_POOL_HEADROOM: usize = 64;

/// One mesh node: a held tokio runtime, an iroh endpoint, and an optional router.
pub struct MeshNode {
    runtime: Runtime,
    endpoint: Endpoint,
    peer: PeerId,
    router: Option<Router>,
    deadline: Duration,
}

impl MeshNode {
    /// Binds a node from `identity`, joining the public iroh network (relays and
    /// DNS address lookup via the n0 preset) so peers can reach it across NATs.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Bind`] when the runtime or endpoint cannot be built.
    pub fn bind(identity: &NodeIdentity) -> MeshResult<Self> {
        Self::bind_with(identity, Binding::Public)
    }

    /// Binds a node for fully offline, direct-address-only use: relays and DNS
    /// disabled, the IP socket pinned to `addr`. This is the testable form, and
    /// the form for a LAN where peers exchange direct [`EndpointAddr`] tickets.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Bind`] when the runtime or endpoint cannot be built.
    pub fn bind_local(identity: &NodeIdentity, addr: std::net::SocketAddr) -> MeshResult<Self> {
        Self::bind_with(identity, Binding::Local(addr))
    }

    fn bind_with(identity: &NodeIdentity, binding: Binding) -> MeshResult<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            // Bound the blocking pool so a flood of long-lived sessions (one
            // pinned blocking thread each) cannot grow the pool without limit.
            // The session semaphore in the handler admits at most
            // MAX_CONCURRENT_SESSIONS of these; the headroom covers the runtime's
            // own short-lived spawn_blocking work.
            .max_blocking_threads(MAX_CONCURRENT_SESSIONS + BLOCKING_POOL_HEADROOM)
            .build()
            .map_err(|err| MeshError::Bind(err.to_string()))?;
        let secret = secret_key_for(identity);
        let endpoint = runtime
            .block_on(build_endpoint(secret, binding))
            .map_err(MeshError::Bind)?;
        let peer = peer_id_for(endpoint.id());
        Ok(Self {
            runtime,
            endpoint,
            peer,
            router: None,
            deadline: DEFAULT_OP_DEADLINE,
        })
    }

    /// Overrides the per-operation stream deadline (default
    /// [`DEFAULT_OP_DEADLINE`]).
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Returns this node's verified [`PeerId`] (its ed25519 public key).
    #[must_use]
    pub fn peer_id(&self) -> PeerId {
        self.peer
    }

    /// Returns a runtime [`Handle`] for the node's owned runtime.
    #[must_use]
    pub fn handle(&self) -> Handle {
        self.runtime.handle().clone()
    }

    /// Returns the node's dialable [`EndpointAddr`] ticket: its id plus the
    /// direct addresses and relay it currently knows.
    ///
    /// For a public node this includes the home relay (populated after the node
    /// comes online); for a local node it includes only the bound IP addresses.
    #[must_use]
    pub fn ticket(&self) -> EndpointAddr {
        self.runtime.block_on(async {
            use iroh::Watcher;
            self.endpoint.watch_addr().get()
        })
    }

    /// Waits until the node has working network connectivity, bounded by
    /// `timeout`, so a freshly bound public node is dialable before it prints a
    /// ticket. Returns whether connectivity was observed in time.
    ///
    /// `online().await` does not by itself guarantee dialability for ~2s
    /// (iroh issue #3713), so callers should still prefer dialing with a ticket
    /// carrying direct addresses for first contact.
    pub fn wait_online(&self, timeout: Duration) -> bool {
        self.runtime
            .block_on(async { tokio::time::timeout(timeout, self.endpoint.online()).await })
            .is_ok()
    }

    /// Starts serving `config` over ALPN [`crate::WANIX_9P_ALPN`].
    ///
    /// The spawned [`Router`] is held by the node; dropping the node shuts it
    /// down. Serving and dialing share the one endpoint and identity.
    pub fn serve(&mut self, config: ServeConfig) {
        let handler = P9ProtocolHandler::new(
            config.with_deadline(self.deadline),
            self.runtime.handle().clone(),
        );
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_9P_ALPN, handler)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Returns a [`MeshDialer`] over this node's endpoint and runtime.
    #[must_use]
    pub fn dialer(&self) -> MeshDialer {
        MeshDialer::new(self.endpoint.clone(), self.runtime.handle().clone())
            .with_deadline(self.deadline)
    }
}

impl Drop for MeshNode {
    fn drop(&mut self) {
        if let Some(router) = self.router.take() {
            // Best-effort graceful shutdown; ignore errors during teardown.
            let _ = self.runtime.block_on(router.shutdown());
        }
    }
}

/// How a node's endpoint reaches the network.
enum Binding {
    /// Public iroh network: relays and DNS address lookup (the n0 preset).
    Public,
    /// Offline, direct-address-only, pinned to one IP socket.
    Local(std::net::SocketAddr),
}

/// Builds and binds the iroh endpoint for `binding`.
async fn build_endpoint(secret: iroh::SecretKey, binding: Binding) -> Result<Endpoint, String> {
    let builder = match binding {
        Binding::Public => Endpoint::builder(presets::N0)
            .secret_key(secret)
            .alpns(vec![crate::WANIX_9P_ALPN.to_vec()]),
        Binding::Local(addr) => Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(vec![crate::WANIX_9P_ALPN.to_vec()])
            .relay_mode(iroh::RelayMode::Disabled)
            .bind_addr(addr)
            .map_err(|err| err.to_string())?,
    };
    builder.bind().await.map_err(|err| err.to_string())
}
