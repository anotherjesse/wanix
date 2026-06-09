//! Outbound side: dial a peer over QUIC and import its namespace.
//!
//! There are two import wires:
//!
//! - [`MeshDialer::dial_native`] (the default mesh path) connects on
//!   [`crate::WANIX_FS_ALPN`] and returns a native-wire [`NativeFs`] over the
//!   held [`Connection`]: one fresh bidi stream per op / per open file, typed
//!   [`wanix_mesh_wire::WireFsError`]s instead of an errno round-trip, and each
//!   never-EOF open file on its own stream.
//! - [`MeshDialer::dial`] (the 9P foreign edge) connects on
//!   [`crate::WANIX_9P_ALPN`], opens one bidi stream, wraps it in a
//!   [`BlockingDuplex`], and hands it to [`wanix_9p_client::RemoteFs::connect`].
//!   `RemoteFs` negotiates `Tversion` over the stream — and because an iroh bidi
//!   stream is invisible to the peer's `accept_bi` until the opener writes its
//!   first byte, that `Tversion` write is what makes the inbound side see the
//!   stream. A dialer that read first would hang. (The same first-write fact
//!   makes the native wire's request frame resolve `accept_bi`.)
//!
//! The connect/open work runs on the mesh runtime; both returned filesystems are
//! fully synchronous [`wanix_fs::FileSystem`]s whose method calls drive the QUIC
//! streams through the held [`Handle`], on non-runtime threads only.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr};
use tokio::runtime::Handle;
use wanix_9p_client::RemoteFs;
use wanix_mesh_wire::{Duplex, NativeFs, StreamFactory};

use crate::duplex::BlockingDuplex;
use crate::error::{MeshError, MeshResult};

/// Dials peers over an iroh [`Endpoint`] and builds importable [`RemoteFs`]es.
#[derive(Clone)]
pub struct MeshDialer {
    endpoint: Endpoint,
    handle: Handle,
    deadline: Option<Duration>,
}

impl MeshDialer {
    /// Creates a dialer over `endpoint`, driving streams on `handle`.
    #[must_use]
    pub fn new(endpoint: Endpoint, handle: Handle) -> Self {
        Self {
            endpoint,
            handle,
            deadline: None,
        }
    }

    /// Sets the per-operation deadline applied to dialed streams.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Dials `addr`, negotiates a 9P session, and returns the remote filesystem.
    ///
    /// The returned [`RemoteFs`] can be bound into a [`wanix_vfs::Namespace`] at
    /// `/n/<node>`; every namespace operation that resolves into it becomes a 9P
    /// exchange over the QUIC stream.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Dial`] when the QUIC connection or stream cannot be
    /// established, and [`MeshError::Session`] when 9P negotiation fails.
    pub fn dial(&self, addr: EndpointAddr) -> MeshResult<Arc<RemoteFs>> {
        self.dial_attach(addr, "")
    }

    /// Dials `addr` and attaches the named subtree `aname`.
    ///
    /// A grant-gated server keys authorization on the verified peer id *and* the
    /// attach name, so importing a scoped capability requires sending the
    /// matching `aname` (e.g. `projects/foo`). [`Self::dial`] sends the empty
    /// root `aname` for the unscoped case.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Dial`] when the QUIC connection or stream cannot be
    /// established, and [`MeshError::Session`] when 9P negotiation fails
    /// (including a default-deny `EACCES` attach rejection).
    pub fn dial_attach(&self, addr: EndpointAddr, aname: &str) -> MeshResult<Arc<RemoteFs>> {
        let duplex = self.open_stream(addr)?;
        let remote = RemoteFs::connect_with_aname(Box::new(duplex), aname)
            .map_err(|err| MeshError::Session(err.to_string()))?;
        Ok(Arc::new(remote))
    }

    /// Dials `addr` over [`crate::WANIX_FS_ALPN`] and returns the imported peer
    /// namespace as a native-wire [`NativeFs`].
    ///
    /// Unlike [`Self::dial`] (9P, one shared serial connection), the native wire
    /// opens **one fresh bidi stream per op / per open file** over a single held
    /// [`Connection`]. The returned [`NativeFs`] is a synchronous
    /// [`wanix_fs::FileSystem`] that can be bound into a [`wanix_vfs::Namespace`]
    /// at `/n/<node>`; every op opens a stream, frames one `postcard` request,
    /// and reads one reply — with typed [`wanix_mesh_wire::WireFsError`]s, not an
    /// errno round-trip, and with each never-EOF open file on its own stream so
    /// it cannot stall any sibling op.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Dial`] when the QUIC connection cannot be established.
    pub fn dial_native(&self, addr: EndpointAddr) -> MeshResult<Arc<NativeFs<IrohStreamFactory>>> {
        self.dial_native_attach(addr, "")
    }

    /// Dials `addr` over the native plane, reserving `aname` for the future
    /// scoped-attach path.
    ///
    /// The native wire binds the principal from the verified `remote_id()` and
    /// the v1 server resolves the connection root from that principal alone
    /// (`AttachPolicy` evaluated at the root attach name), so a default-deny
    /// rejection surfaces lazily as a per-op transport fault rather than at dial
    /// time. `aname` is threaded for symmetry with [`Self::dial_attach`] and a
    /// future per-attach scoping path; v1 does not yet carry it on the wire.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Dial`] when the QUIC connection cannot be established.
    pub fn dial_native_attach(
        &self,
        addr: EndpointAddr,
        aname: &str,
    ) -> MeshResult<Arc<NativeFs<IrohStreamFactory>>> {
        let _ = aname;
        // Connect once up front so an unreachable peer fails the dial fast. The
        // factory keeps the endpoint + addr so it can RE-dial if this connection
        // later dies (the peer restarts): a bare `iroh://PEER` re-resolves through
        // mDNS/relay, and an addr-bearing one falls back to discovery, so the mount
        // recovers by stable id instead of resetting forever.
        let connection = self.connect_native(addr.clone())?;
        let factory = IrohStreamFactory::new(
            self.endpoint.clone(),
            addr,
            connection,
            self.handle.clone(),
            self.deadline,
        );
        Ok(Arc::new(NativeFs::new(factory)))
    }

    /// Connects to `addr` over the native ALPN, returning the held connection the
    /// per-op stream factory opens fresh bidi streams over.
    fn connect_native(&self, addr: EndpointAddr) -> MeshResult<Connection> {
        let endpoint = self.endpoint.clone();
        block_on_deadline(&self.handle, self.deadline, "native connect", async move {
            endpoint.connect(addr, crate::WANIX_FS_ALPN).await
        })
        .map_err(MeshError::Dial)
    }

    /// Connects and opens one bidi stream, returning the bridged duplex.
    fn open_stream(&self, addr: EndpointAddr) -> MeshResult<BlockingDuplex> {
        let endpoint = self.endpoint.clone();
        let connection = block_on_deadline(&self.handle, self.deadline, "9P connect", async move {
            endpoint.connect(addr, crate::WANIX_9P_ALPN).await
        })
        .map_err(MeshError::Dial)?;
        let (send, recv) =
            block_on_deadline(&self.handle, self.deadline, "9P open_bi", async move {
                connection.open_bi().await
            })
            .map_err(MeshError::Dial)?;
        Ok(BlockingDuplex::new(
            send,
            recv,
            self.handle.clone(),
            self.deadline,
        ))
    }
}

/// The native-wire [`StreamFactory`]: opens a fresh iroh bidi stream per op,
/// re-dialing the peer if the cached connection dies.
///
/// This is the mesh's implementation of the wire crate's transport seam. It
/// caches one iroh [`Connection`] and, for each native-wire op (`open_stream`),
/// opens a new bidi stream on it and wraps it in the existing [`BlockingDuplex`],
/// so `wanix-mesh-wire` only ever sees the sync [`Duplex`] and stays iroh/tokio
/// free.
///
/// The cached connection is to ONE server process. When that process restarts
/// (or the link resets), the next `open_bi` fails; rather than resetting forever,
/// the factory re-dials the peer by its [`EndpointAddr`] — mDNS/relay/direct
/// re-resolves the restarted peer by its stable id — caches the fresh connection,
/// and retries the stream. A `generation` counter collapses a herd of concurrent
/// reconnects into one re-dial.
#[derive(Clone)]
pub struct IrohStreamFactory {
    endpoint: Endpoint,
    addr: EndpointAddr,
    shared: Arc<Mutex<SharedConnection>>,
    handle: Handle,
    deadline: Option<Duration>,
}

/// The cached connection plus a generation that bumps on every re-dial, so a
/// concurrent reconnect can tell whether another op already replaced it.
struct SharedConnection {
    connection: Connection,
    generation: u64,
}

impl IrohStreamFactory {
    fn new(
        endpoint: Endpoint,
        addr: EndpointAddr,
        connection: Connection,
        handle: Handle,
        deadline: Option<Duration>,
    ) -> Self {
        Self {
            endpoint,
            addr,
            shared: Arc::new(Mutex::new(SharedConnection {
                connection,
                generation: 0,
            })),
            handle,
            deadline,
        }
    }

    /// The currently-cached connection and its generation.
    fn snapshot(&self) -> (Connection, u64) {
        let shared = self.shared.lock().expect("connection mutex poisoned");
        (shared.connection.clone(), shared.generation)
    }

    /// Re-dials the peer and caches the fresh connection, unless another op
    /// already reconnected since `stale_generation` (in which case its connection
    /// is returned). The re-dial re-resolves the route by stable peer id.
    fn reconnect(&self, stale_generation: u64) -> std::io::Result<Connection> {
        let mut shared = self.shared.lock().expect("connection mutex poisoned");
        if shared.generation != stale_generation {
            return Ok(shared.connection.clone());
        }
        let endpoint = self.endpoint.clone();
        let addr = self.addr.clone();
        let connection = block_on_deadline(
            &self.handle,
            self.deadline,
            "native reconnect",
            async move { endpoint.connect(addr, crate::WANIX_FS_ALPN).await },
        )
        .map_err(|err| std::io::Error::other(format!("native reconnect failed: {err}")))?;
        shared.connection = connection.clone();
        shared.generation = shared.generation.wrapping_add(1);
        Ok(connection)
    }

    /// Opens a fresh bidi stream on `connection` and wraps it as a [`Duplex`].
    fn open_bi_on(&self, connection: &Connection) -> std::io::Result<Box<dyn Duplex>> {
        let connection = connection.clone();
        // Open the bidi stream on the held runtime. The first byte the wire crate
        // writes (its request frame) is what makes the peer's `accept_bi`
        // resolve, exactly as the 9P `Tversion` write does today.
        let (send, recv) =
            block_on_deadline(&self.handle, self.deadline, "native open_bi", async move {
                connection.open_bi().await
            })
            .map_err(|err| std::io::Error::other(format!("native dial open_bi failed: {err}")))?;
        // The client keeps the deadline on both halves: its read always follows a
        // request write, so the per-op deadline is a sane response timeout.
        Ok(Box::new(BlockingDuplex::new(
            send,
            recv,
            self.handle.clone(),
            self.deadline,
        )))
    }
}

impl StreamFactory for IrohStreamFactory {
    fn open_stream(&self) -> std::io::Result<Box<dyn Duplex>> {
        let (connection, generation) = self.snapshot();
        match self.open_bi_on(&connection) {
            Ok(duplex) => Ok(duplex),
            // The cached connection likely died (peer restart / link reset).
            // Re-dial once and retry the stream on the fresh connection; the next
            // op self-heals again if the peer is still down.
            Err(_) => {
                let fresh = self.reconnect(generation)?;
                self.open_bi_on(&fresh)
            }
        }
    }
}

fn block_on_deadline<F, T, E>(
    handle: &Handle,
    deadline: Option<Duration>,
    operation: &str,
    future: F,
) -> Result<T, String>
where
    F: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    handle.block_on(async move {
        let result = match deadline {
            Some(deadline) => tokio::time::timeout(deadline, future)
                .await
                .map_err(|_| format!("{operation} timed out after {}ms", deadline.as_millis()))?,
            None => future.await,
        };
        result.map_err(|err| err.to_string())
    })
}
