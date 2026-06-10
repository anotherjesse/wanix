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

use iroh::endpoint::{IdleTimeout, QuicTransportConfig, presets};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr};
use tokio::runtime::{Handle, Runtime};
use wanix_id::{NodeIdentity, PeerId};

use crate::dialer::MeshDialer;
use crate::error::{MeshError, MeshResult};
use crate::handler::{P9ProtocolHandler, ServeConfig};
use crate::identity::{peer_id_for, secret_key_for};
use crate::wire_handler::{NativeFsHandler, NativeServeConfig};

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

/// Hard cap on concurrently running cpu jobs on the acceptor side.
///
/// The cpu plane shares the runtime's blocking pool with the 9P plane, and a
/// running job pins blocking-pool threads (the `run_job` thread parked in
/// `block_on` on the reverse-export reader, plus transient role-sort work). With
/// no cap a hostile-but-allowlisted peer opening many stalled cpu connections
/// could exhaust the pool and starve the 9P plane. This bound mirrors the 9P
/// session semaphore for the exec plane; the blocking pool is sized to admit
/// these jobs (each accounted at [`CPU_THREADS_PER_JOB`]) alongside 9P sessions.
pub const MAX_CONCURRENT_CPU_JOBS: usize = 32;

/// Blocking-pool threads a single in-flight cpu job is accounted at.
///
/// A running job parks one blocking-pool thread inside `run_job`'s `block_on`
/// on the reverse-export reader for the job's lifetime; the role-sort step uses
/// a second, transient `spawn_blocking`. Sizing the pool at two threads per
/// admitted job keeps a full cpu plane from borrowing from the 9P budget.
const CPU_THREADS_PER_JOB: usize = 2;

/// Blocking-pool headroom above the admitted 9P sessions and cpu jobs for the
/// runtime's own transient `spawn_blocking` work.
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

    /// Binds a node for local/LAN use: relays and the public DNS address lookup
    /// disabled, the IP socket pinned to `addr`. mDNS local discovery is still on
    /// (see [`build_endpoint`]), so LAN peers can reach it by a bare `iroh://PEER`
    /// as well as via an exchanged direct [`EndpointAddr`] ticket. The testable
    /// form, and the form for a LAN.
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
            // Bound the blocking pool so a flood of long-lived sessions or cpu
            // jobs (each pinning blocking threads) cannot grow the pool without
            // limit. The session semaphore in the 9P handler admits at most
            // MAX_CONCURRENT_SESSIONS of these and the cpu acceptor's semaphore
            // admits at most MAX_CONCURRENT_CPU_JOBS (accounted at
            // CPU_THREADS_PER_JOB each); the headroom covers the runtime's own
            // short-lived spawn_blocking work.
            .max_blocking_threads(
                MAX_CONCURRENT_SESSIONS
                    + MAX_CONCURRENT_CPU_JOBS * CPU_THREADS_PER_JOB
                    + BLOCKING_POOL_HEADROOM,
            )
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

    /// Returns a clone of the node's [`Endpoint`].
    ///
    /// The endpoint is shared by every plane: the 9P [`Router`], the blob
    /// downloader, and any directly opened bi-stream all ride this one
    /// identity-bound endpoint, which is what lets the data plane and control
    /// plane share a peer.
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.endpoint.clone()
    }

    /// Builds an in-memory content-addressed store ([`crate::IrohCasStore`]) over
    /// this node's endpoint and runtime, fetching missing blobs from `peers`.
    ///
    /// `peers` are full provider tickets ([`EndpointAddr`]); pass the
    /// [`Self::ticket`] of each node whose blobs this store should fetch. The
    /// store shares the node's identity-bound endpoint, so blobs it serves and
    /// fetches travel the same QUIC path as 9P. Register its blobs protocol with
    /// [`Self::serve_with_blobs`] to make this node a provider.
    #[must_use]
    pub fn cas_store(&self, peers: Vec<EndpointAddr>) -> crate::IrohCasStore {
        crate::IrohCasStore::memory(self.endpoint.clone(), self.runtime.handle().clone(), peers)
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
        let handler = self.p9_handler(config);
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_9P_ALPN, handler)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Starts serving `config` over the **native** wire on
    /// [`crate::WANIX_FS_ALPN`].
    ///
    /// This is the native analog of [`Self::serve`]: a hand-rolled `postcard`
    /// frame, one bidi stream per op / per open file, the full
    /// [`wanix_fs::FileSystem`] trait with typed errors, and a per-connection
    /// principal bound from `remote_id()`. The spawned [`Router`] is held by the
    /// node; dropping the node shuts it down. Serving and dialing share the one
    /// endpoint and identity, and the endpoint advertises both ALPNs, so a node
    /// can speak the 9P and native wires at once during the transition.
    pub fn serve_native(&mut self, config: NativeServeConfig) {
        let handler = self.native_handler(config);
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_FS_ALPN, handler)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Starts serving 9P (`config`) on [`crate::WANIX_9P_ALPN`] *and* the blob
    /// data plane (`cas`) on [`crate::BLOBS_ALPN`] from one shared [`Router`].
    ///
    /// This is the blueprint's "one endpoint, one identity, two planes": the
    /// control plane (9P walk/stat/mutation) and the data plane (BLAKE3 bulk
    /// blobs) accept on the same endpoint, so a peer reaches both over a single
    /// QUIC path. The spawned router advertises both ALPNs (iroh's
    /// `Router::spawn` sets the endpoint ALPNs from its accepted handlers).
    pub fn serve_with_blobs(&mut self, config: ServeConfig, cas: &crate::IrohCasStore) {
        let handler = self.p9_handler(config);
        let blobs = crate::blobs_protocol(cas);
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_9P_ALPN, handler)
                .accept(crate::BLOBS_ALPN, blobs)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Starts serving the **native** control plane (`config`) on
    /// [`crate::WANIX_FS_ALPN`] *and* the blob data plane (`cas`) on
    /// [`crate::BLOBS_ALPN`] from one shared [`Router`].
    ///
    /// The native analog of [`Self::serve_with_blobs`]: the Wanix↔Wanix mesh
    /// control plane (typed `FsError`s, one bidi stream per op / per open file)
    /// and the BLAKE3 bulk-blob data plane accept on the same identity-bound
    /// endpoint. 9P is not advertised, so a node that wants only the native
    /// control plane beside its blob plane serves exactly these two ALPNs.
    pub fn serve_native_with_blobs(
        &mut self,
        config: NativeServeConfig,
        cas: &crate::IrohCasStore,
    ) {
        let handler = self.native_handler(config);
        let blobs = crate::blobs_protocol(cas);
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_FS_ALPN, handler)
                .accept(crate::BLOBS_ALPN, blobs)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Builds a [`crate::GossipPlumbPort`] over this node's endpoint and runtime.
    ///
    /// `bootstrap` are dialable peer addresses ([`EndpointAddr`] tickets) a
    /// freshly joined topic dials to enter the swarm (pass the [`Self::ticket`]
    /// of nodes already on the bus); an empty list still serves same-node
    /// subscribers and accepts inbound gossip once the gossip ALPN is registered
    /// with [`Self::serve_with_plumb`]. The returned port backs a
    /// [`wanix_plumb::PlumbDevice`] so a `#plumb/<topic>` write crosses the mesh.
    #[must_use]
    pub fn plumb_port(&self, bootstrap: Vec<EndpointAddr>) -> crate::GossipPlumbPort {
        crate::GossipPlumbPort::new(
            self.endpoint.clone(),
            self.runtime.handle().clone(),
            bootstrap,
        )
    }

    /// Starts serving 9P (`config`) on [`crate::WANIX_9P_ALPN`] *and* the gossip
    /// coordination plane (`plumb`) on [`crate::GOSSIP_ALPN`] from one [`Router`].
    ///
    /// This adds the third plane to the node: control (9P), and now coordination
    /// (gossip). Unlike the exec planes, gossip grants no filesystem or code
    /// capability, so it is safe to advertise on the public endpoint alongside a
    /// grant-gated 9P serve.
    pub fn serve_with_plumb(&mut self, config: ServeConfig, plumb: &crate::GossipPlumbPort) {
        let handler = self.p9_handler(config);
        let gossip = plumb.protocol();
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_9P_ALPN, handler)
                .accept(crate::GOSSIP_ALPN, gossip)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Starts serving the **native** control plane (`config`) on
    /// [`crate::WANIX_FS_ALPN`] *and* the gossip coordination plane (`plumb`) on
    /// [`crate::GOSSIP_ALPN`] from one shared [`Router`].
    ///
    /// The native analog of [`Self::serve_with_plumb`]: the Wanix↔Wanix mesh
    /// control plane beside the broker-less gossip bus, both on one
    /// identity-bound endpoint, with 9P not advertised. The gossip plane is a
    /// distinct ALPN from the FileSystem control plane and is unchanged by the
    /// native wire — it carries plumber envelopes, not `FileSystem` ops.
    pub fn serve_native_with_plumb(
        &mut self,
        config: NativeServeConfig,
        plumb: &crate::GossipPlumbPort,
    ) {
        let handler = self.native_handler(config);
        let gossip = plumb.protocol();
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_FS_ALPN, handler)
                .accept(crate::GOSSIP_ALPN, gossip)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Starts serving the cpu exec plane (`acceptor`) on [`crate::WANIX_CPU_ALPN`].
    ///
    /// The cpu plane runs grant-allowlisted remote tasks against a caller's
    /// reverse-exported namespace. It is a distinct ALPN from the 9P plane and is
    /// gated by the acceptor's own allowlist, per the blueprint's "exec-device
    /// export stays local-trust until public auth lands": a node should serve cpu
    /// only on a direct-address-only (local-trust) endpoint, never the public one.
    pub fn serve_cpu(&mut self, acceptor: crate::CpuAcceptor) {
        let router = self.runtime.block_on(async {
            Router::builder(self.endpoint.clone())
                .accept(crate::WANIX_CPU_ALPN, acceptor)
                .spawn()
        });
        self.router = Some(router);
    }

    /// Builds a [`crate::CpuAcceptor`] over this node's runtime handle.
    ///
    /// `table_factory` produces a fresh driver-registered task table per job;
    /// `allowlist` is the exec trust gate (`true` admits a peer to run code).
    #[must_use]
    pub fn cpu_acceptor(
        &self,
        table_factory: crate::TaskTableFactory,
        allowlist: std::sync::Arc<dyn Fn(PeerId) -> bool + Send + Sync>,
    ) -> crate::CpuAcceptor {
        crate::CpuAcceptor::new(
            table_factory,
            allowlist,
            self.runtime.handle().clone(),
            self.deadline,
        )
    }

    /// Dials `addr` over the cpu ALPN and returns a [`crate::CpuDialer`].
    ///
    /// The returned dialer runs one job: it opens the control and export streams,
    /// exports a scoped reverse namespace, and drains the result batch.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Dial`] when the QUIC connection cannot be established.
    pub fn dial_cpu(&self, addr: EndpointAddr) -> MeshResult<crate::CpuDialer> {
        let endpoint = self.endpoint.clone();
        let connection = self
            .runtime
            .block_on(async move { endpoint.connect(addr, crate::WANIX_CPU_ALPN).await })
            .map_err(|err| MeshError::Dial(err.to_string()))?;
        Ok(crate::CpuDialer::new(
            connection,
            self.runtime.handle().clone(),
            self.deadline,
        ))
    }

    /// Builds the deadline-bound 9P protocol handler for `config`.
    fn p9_handler(&self, config: ServeConfig) -> P9ProtocolHandler {
        P9ProtocolHandler::new(
            config.with_deadline(self.deadline),
            self.runtime.handle().clone(),
        )
    }

    /// Builds the deadline-bound native-wire protocol handler for `config`.
    fn native_handler(&self, config: NativeServeConfig) -> NativeFsHandler {
        NativeFsHandler::new(
            config.with_deadline(self.deadline),
            self.runtime.handle().clone(),
        )
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

/// The ALPNs a Wanix endpoint advertises at bind time.
///
/// A node speaks both control-plane wires: the 9P plane (the foreign-edge
/// gateway) and the native `wanix-mesh-wire` plane (the Wanix↔Wanix mesh path).
/// Both ride one identity-bound endpoint, so the bind-time ALPN set lists both;
/// `Router::spawn` further unions in whatever the registered handlers accept.
fn endpoint_alpns() -> Vec<Vec<u8>> {
    vec![crate::WANIX_9P_ALPN.to_vec(), crate::WANIX_FS_ALPN.to_vec()]
}

/// QUIC keep-alive interval on every Wanix endpoint (see
/// [`endpoint_transport_config`]).
const ENDPOINT_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(5);

/// QUIC max idle timeout on every Wanix endpoint (see
/// [`endpoint_transport_config`]).
const ENDPOINT_MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// The explicit liveness posture of every Wanix endpoint: keep-alives every
/// [`ENDPOINT_KEEP_ALIVE_INTERVAL`], connections idle-timed-out after
/// [`ENDPOINT_MAX_IDLE_TIMEOUT`].
///
/// This is what bounds detection of a hard-killed peer: a subscriber that
/// vanishes without closing (power loss, SIGKILL, vanished network) stops
/// acking keep-alives and its connection — with every server-side stream,
/// session permit, and blocking-pool thread pinned by it — is torn down
/// within the idle timeout instead of leaking until process exit. Pinned
/// here as an explicit contract rather than inherited from iroh defaults.
///
/// Residual (documented, not fixed): a *live but idle* subscriber answers
/// keep-alives forever, so a parked never-EOF read (an appfs `stream`
/// subscription, a `#plumb` recv) still pins one session permit and one
/// blocking-pool thread per open file for as long as the client keeps the
/// file open. Bounded by [`MAX_CONCURRENT_SESSIONS`]; making parked reads
/// permit-free is ADR 0008 liveness follow-up work.
fn endpoint_transport_config() -> QuicTransportConfig {
    let idle = IdleTimeout::try_from(ENDPOINT_MAX_IDLE_TIMEOUT)
        .expect("endpoint idle timeout fits the QUIC varint range");
    QuicTransportConfig::builder()
        .keep_alive_interval(ENDPOINT_KEEP_ALIVE_INTERVAL)
        .max_idle_timeout(Some(idle))
        .build()
}

/// Builds and binds the iroh endpoint for `binding`.
///
/// Every endpoint enables mDNS local-network address lookup (advertise +
/// resolve), so a peer is reachable by a bare `iroh://PEER` on the LAN/same
/// machine without a direct `addr=` hint — and a restarted node on a new port is
/// rediscovered by its stable id. This is on by default for both bindings;
/// `Public` keeps N0 DNS/Pkarr too, `Local` keeps relays disabled. (An opt-out
/// can be added later if an environment without multicast needs it.)
async fn build_endpoint(secret: iroh::SecretKey, binding: Binding) -> Result<Endpoint, String> {
    let builder = match binding {
        Binding::Public => Endpoint::builder(presets::N0)
            .secret_key(secret)
            .alpns(endpoint_alpns()),
        Binding::Local(addr) => Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(endpoint_alpns())
            .relay_mode(iroh::RelayMode::Disabled)
            .bind_addr(addr)
            .map_err(|err| err.to_string())?,
    };
    let builder = builder
        .transport_config(endpoint_transport_config())
        .address_lookup(iroh_mdns_address_lookup::MdnsAddressLookup::builder().advertise(true));
    builder.bind().await.map_err(|err| err.to_string())
}
