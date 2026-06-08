//! Outbound side: dial a peer over QUIC and import its namespace.
//!
//! [`MeshDialer::dial`] connects to a peer's [`iroh::EndpointAddr`], opens one
//! bidi stream, wraps it in a [`BlockingDuplex`], and hands it to
//! [`wanix_9p_client::RemoteFs::connect`]. `RemoteFs` then negotiates `Tversion`
//! over the stream — and because an iroh bidi stream is invisible to the peer's
//! `accept_bi` until the opener writes its first byte, that `Tversion` write is
//! exactly what makes the inbound side see the stream. A dialer that read first
//! would hang.
//!
//! The connect/open work runs on the mesh runtime; the returned `RemoteFs` is a
//! fully synchronous [`wanix_fs::FileSystem`] whose method calls drive the QUIC
//! stream through the held [`Handle`], on non-runtime threads only.

use std::sync::Arc;
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

    /// Dials `addr`/`aname` and wraps the import so blocking streaming opens get
    /// their own bidi stream, returning a [`crate::StreamingImportFs`].
    ///
    /// This is the deadlock-safe form for importing a peer's service namespace:
    /// the everyday-ops connection serves walk/stat/readdir/mutation and short
    /// reads, while an open of a never-EOF service file (an `#agent` event/reply
    /// stream, a `#plumb` recv stream — see [`crate::default_blocking_stream`])
    /// dials a fresh stream so it cannot freeze the rest of the import.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::Dial`]/[`MeshError::Session`] when the everyday-ops
    /// connection cannot be established or negotiated.
    pub fn dial_streaming(
        &self,
        addr: EndpointAddr,
        aname: &str,
        predicate: crate::StreamPredicate,
    ) -> MeshResult<crate::StreamingImportFs> {
        let shared = self.dial_attach(addr.clone(), aname)?;
        Ok(crate::StreamingImportFs::new(
            shared,
            self.clone(),
            addr,
            aname,
            predicate,
        ))
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
        let connection = self.connect_native(addr)?;
        let factory = IrohStreamFactory {
            connection,
            handle: self.handle.clone(),
            deadline: self.deadline,
        };
        Ok(Arc::new(NativeFs::new(factory)))
    }

    /// Connects to `addr` over the native ALPN, returning the held connection the
    /// per-op stream factory opens fresh bidi streams over.
    fn connect_native(&self, addr: EndpointAddr) -> MeshResult<Connection> {
        let endpoint = self.endpoint.clone();
        self.handle
            .block_on(async move { endpoint.connect(addr, crate::WANIX_FS_ALPN).await })
            .map_err(|err| MeshError::Dial(err.to_string()))
    }

    /// Connects and opens one bidi stream, returning the bridged duplex.
    fn open_stream(&self, addr: EndpointAddr) -> MeshResult<BlockingDuplex> {
        let endpoint = self.endpoint.clone();
        let (send, recv) = self
            .handle
            .block_on(async move {
                let connection = endpoint.connect(addr, crate::WANIX_9P_ALPN).await?;
                connection.open_bi().await.map_err(Into::into)
            })
            .map_err(|err: DialError| MeshError::Dial(err.to_string()))?;
        Ok(BlockingDuplex::new(
            send,
            recv,
            self.handle.clone(),
            self.deadline,
        ))
    }
}

/// The native-wire [`StreamFactory`]: opens a fresh iroh bidi stream per op.
///
/// This is the mesh's implementation of the wire crate's transport seam. It
/// holds one iroh [`Connection`] and the node's runtime [`Handle`], and for each
/// native-wire op (`open_stream`) opens a new bidi stream on that connection and
/// wraps it in the existing [`BlockingDuplex`], so `wanix-mesh-wire` only ever
/// sees the sync [`Duplex`] and stays iroh/tokio-free. The held connection must
/// outlive every op (the [`BlockingDuplex`] holds a non-owning [`Handle`]); the
/// owning [`crate::MeshNode`]/import keeps it alive.
#[derive(Clone)]
pub struct IrohStreamFactory {
    connection: Connection,
    handle: Handle,
    deadline: Option<Duration>,
}

impl StreamFactory for IrohStreamFactory {
    fn open_stream(&self) -> std::io::Result<Box<dyn Duplex>> {
        let connection = self.connection.clone();
        // Open the bidi stream on the held runtime. The first byte the wire crate
        // writes (its request frame) is what makes the peer's `accept_bi`
        // resolve, exactly as the 9P `Tversion` write does today.
        let (send, recv) = self
            .handle
            .block_on(async move { connection.open_bi().await })
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

/// The connect/open failure surface, unifying iroh's connect and stream errors.
#[derive(Debug)]
enum DialError {
    /// The QUIC connection could not be established.
    Connect(iroh::endpoint::ConnectError),
    /// The bidirectional stream could not be opened on the connection.
    OpenStream(iroh::endpoint::ConnectionError),
}

impl std::fmt::Display for DialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(err) => write!(f, "{err}"),
            Self::OpenStream(err) => write!(f, "{err}"),
        }
    }
}

impl From<iroh::endpoint::ConnectError> for DialError {
    fn from(err: iroh::endpoint::ConnectError) -> Self {
        Self::Connect(err)
    }
}

impl From<iroh::endpoint::ConnectionError> for DialError {
    fn from(err: iroh::endpoint::ConnectionError) -> Self {
        Self::OpenStream(err)
    }
}
