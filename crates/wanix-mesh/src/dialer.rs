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

use iroh::{Endpoint, EndpointAddr};
use tokio::runtime::Handle;
use wanix_9p_client::RemoteFs;

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
