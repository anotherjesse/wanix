//! Inbound side: serve a Wanix namespace over QUIC, gated by verified identity.
//!
//! [`P9ProtocolHandler`] is the iroh [`ProtocolHandler`] for ALPN
//! [`crate::WANIX_9P_ALPN`]. On each accepted connection it reads the
//! cryptographically verified peer id from the QUIC handshake (never a
//! client-claimed `uname`), then for every accepted bidi stream runs the
//! unchanged synchronous [`P9Server::serve_stream`] inside `spawn_blocking`,
//! bridging the split async stream halves through [`BlockingReader`] and
//! [`BlockingWriter`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use wanix_9p::{AttachPolicy, P9Server, PeerId};
use wanix_fs::FileSystem;

use crate::duplex::{BlockingReader, BlockingWriter};
use crate::identity::peer_id_for;
use crate::node::MAX_CONCURRENT_SESSIONS;

/// The default-root and policy a handler installs for every authorized peer.
///
/// The root is the served Wanix namespace; the policy decides, per verified
/// peer and attach name, which scoped [`wanix_vfs::SubtreeFs`] the peer attaches.
/// When `policy` is `None` the served root is exported wholesale (loopback /
/// fully trusted), exactly as an unguarded `p9-listen`.
#[derive(Clone)]
pub struct ServeConfig {
    root: Arc<dyn FileSystem>,
    policy: Option<Arc<dyn AttachPolicy>>,
    deadline: Option<Duration>,
}

impl ServeConfig {
    /// Serves `root` to every peer with no per-peer grant gate.
    #[must_use]
    pub fn open(root: Arc<dyn FileSystem>) -> Self {
        Self {
            root,
            policy: None,
            deadline: None,
        }
    }

    /// Serves `root` but authorizes every attach through `policy`.
    #[must_use]
    pub fn guarded(root: Arc<dyn FileSystem>, policy: Arc<dyn AttachPolicy>) -> Self {
        Self {
            root,
            policy: Some(policy),
            deadline: None,
        }
    }

    /// Sets the per-operation deadline applied to each served stream.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }
}

/// The iroh protocol handler that exports a Wanix namespace as 9P over QUIC.
#[derive(Clone)]
pub struct P9ProtocolHandler {
    config: ServeConfig,
    handle: Handle,
    /// Caps concurrently served sessions; one permit is held per live stream for
    /// the stream's lifetime, so the blocking pool cannot be exhausted by a
    /// slow-peer flood of half-open mounts.
    sessions: Arc<Semaphore>,
}

impl P9ProtocolHandler {
    /// Builds a handler that serves `config`, driving streams on `handle`.
    ///
    /// Concurrent sessions are capped at [`MAX_CONCURRENT_SESSIONS`].
    #[must_use]
    pub fn new(config: ServeConfig, handle: Handle) -> Self {
        Self {
            config,
            handle,
            sessions: Arc::new(Semaphore::new(MAX_CONCURRENT_SESSIONS)),
        }
    }
}

impl fmt::Debug for P9ProtocolHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("P9ProtocolHandler")
            .field("guarded", &self.config.policy.is_some())
            .finish()
    }
}

impl ProtocolHandler for P9ProtocolHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        // Identity is read from the verified QUIC handshake. 0-RTT is not used
        // (we never call `into_0rtt`), so `remote_id` is the proven peer key
        // before any Tattach is served.
        let peer = peer_id_for(connection.remote_id());
        loop {
            let (send, recv) = match connection.accept_bi().await {
                Ok(streams) => streams,
                // No more streams: the peer closed the connection. Routine over
                // NAT/relay churn, not an error.
                Err(_) => return Ok(()),
            };
            // Admit at most MAX_CONCURRENT_SESSIONS live sessions. Each holds one
            // blocking-pool thread for its lifetime; the owned permit is moved
            // into the blocking task and released when the session ends. The
            // semaphore is never closed, so acquire only fails on shutdown.
            let Ok(permit) = Arc::clone(&self.sessions).acquire_owned().await else {
                return Ok(());
            };
            let config = self.config.clone();
            let handle = self.handle.clone();
            // serve_stream is synchronous and one-request-at-a-time. Run each
            // stream on a blocking-pool thread so a long-lived session never pins
            // a runtime worker and the BlockingReader/Writer can block_on safely.
            // Detached: every stream serves independently for the connection's
            // life, so a blocked read on one stream never stalls another.
            tokio::task::spawn_blocking(move || {
                serve_one_stream(&config, peer, send, recv, handle);
                // Permit held until the session ends, then released here.
                drop(permit);
            });
        }
    }
}

/// Runs the synchronous 9P server over one bridged stream until EOF or error.
fn serve_one_stream(
    config: &ServeConfig,
    peer: PeerId,
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    handle: Handle,
) {
    let mut server = match &config.policy {
        Some(policy) => P9Server::with_policy(Arc::clone(&config.root), peer, Arc::clone(policy)),
        None => P9Server::new(Arc::clone(&config.root)),
    };
    // The server's read is the *idle* wait for the next client request: 9P is
    // strict request/response with no keepalive, so a healthy, mounted-but-idle
    // session blocks here indefinitely between operations. Applying the per-op
    // deadline to this read would tear down a perfectly alive session after a
    // brief pause (the "open a mount, walk away for a minute" demo). The per-op
    // deadline bounds *in-flight* work, so it rides only on the server's write
    // (sending a response, where a stalled peer must not park a thread forever).
    // The client side keeps the deadline on both halves because its read always
    // follows a request write, making 30s-per-RPC a sane response timeout.
    let reader = BlockingReader::new(recv, handle.clone(), None);
    let writer = BlockingWriter::new(send, handle, config.deadline);
    // serve_stream alternates strictly serially between reader and writer; the
    // two halves are distinct streams so there is no aliasing.
    let _ = server.serve_stream(reader, writer);
}
