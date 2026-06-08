//! Inbound side of the **native** wire: serve a Wanix `FileSystem` over QUIC.
//!
//! [`NativeFsHandler`] is the iroh [`ProtocolHandler`] for ALPN
//! [`crate::WANIX_FS_ALPN`], the structural analog of [`crate::P9ProtocolHandler`]
//! for the native `wanix-mesh-wire` plane. On each accepted connection it binds
//! the principal **once** from the cryptographically verified peer id
//! (`connection.remote_id()`, never a client-claimed name), resolves the
//! per-connection root via the existing [`wanix_id::AttachPolicy`] (default-deny
//! on `None`), and then for every accepted bidi stream runs the unchanged
//! synchronous [`wanix_mesh_wire::serve_one`] inside `spawn_blocking`, bridging
//! the split async stream halves through an asymmetric [`BlockingDuplex`].
//!
//! Three invariants from the design plan (§7, risk register) are honored here:
//!
//! - **Per-connection principal, not a per-op argument.** The principal is bound
//!   to this server-side handler from the transport, never carried on the wire,
//!   so the core `FileSystem`/`File` traits stay principal-blind. This is the
//!   non-no-op `NamespaceProvider` seam: the provider is `AttachPolicy`, and its
//!   result — the per-connection root — is actually used for the connection's
//!   whole life.
//! - **Dispatch into the sync `FileSystem` runs on `spawn_blocking`.** A
//!   streaming read on a never-EOF device blocks; it must block a blocking-pool
//!   thread, never a runtime worker. One [`crate::node::MAX_CONCURRENT_SESSIONS`]
//!   permit is held per live stream (one open file == one pinned thread).
//! - **No deadline on the idle open-file read.** The asymmetric [`BlockingDuplex`]
//!   carries the per-op deadline on writes only; the server's read of the next
//!   `FileOp` is the idle wait of a live subscription and is never timed.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use wanix_fs::FileSystem;
use wanix_id::{AttachPolicy, PeerId};

use crate::duplex::BlockingDuplex;
use crate::identity::peer_id_for;
use crate::node::MAX_CONCURRENT_SESSIONS;

/// The v1 single-attach name the native handler resolves the connection root by.
///
/// The native wire has no `Tattach`/`uname` ceremony: the principal is the
/// verified `remote_id()` and the root is resolved once per connection. For v1
/// that resolution uses the empty/root attach name, exactly as
/// `P9Server::with_policy` installs the empty-`aname` root on the first
/// `Tattach`. A future scoped-attach path would carry the name in the dial.
const ROOT_ANAME: &str = "";

/// The default-root and policy a native handler installs per authorized peer.
///
/// Mirrors the 9P [`crate::ServeConfig`]: the root is the served Wanix
/// `FileSystem`; `policy` decides, per verified peer, which scoped
/// [`wanix_vfs::SubtreeFs`] that peer attaches. When `policy` is `None` the root
/// is exported wholesale (loopback / fully trusted), exactly as an unguarded
/// `p9-listen`.
#[derive(Clone)]
pub struct NativeServeConfig {
    root: Arc<dyn FileSystem>,
    policy: Option<Arc<dyn AttachPolicy>>,
    deadline: Option<Duration>,
}

impl NativeServeConfig {
    /// Serves `root` to every peer with no per-peer grant gate.
    #[must_use]
    pub fn open(root: Arc<dyn FileSystem>) -> Self {
        Self {
            root,
            policy: None,
            deadline: None,
        }
    }

    /// Serves `root` but authorizes the connection root through `policy`.
    #[must_use]
    pub fn guarded(root: Arc<dyn FileSystem>, policy: Arc<dyn AttachPolicy>) -> Self {
        Self {
            root,
            policy: Some(policy),
            deadline: None,
        }
    }

    /// Sets the per-operation deadline applied to in-flight writes on each stream.
    #[must_use]
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Resolves the per-connection root for a verified `peer`.
    ///
    /// With no policy the wholesale root is served. With a policy the root is the
    /// scoped [`wanix_id::Authorization`] root for `peer` attaching
    /// [`ROOT_ANAME`]; `None` (default-deny) returns `None` so the caller refuses
    /// the connection — the native analog of the 9P `EACCES` attach rejection.
    fn resolve_root(&self, peer: PeerId) -> Option<Arc<dyn FileSystem>> {
        match &self.policy {
            None => Some(Arc::clone(&self.root)),
            Some(policy) => policy
                .evaluate(peer, ROOT_ANAME)
                .map(|authorization| authorization.root),
        }
    }
}

/// The iroh protocol handler that exports a Wanix `FileSystem` over the native
/// `wanix-mesh-wire` plane.
#[derive(Clone)]
pub struct NativeFsHandler {
    config: NativeServeConfig,
    handle: Handle,
    /// Caps concurrently served streams; one permit is held per live stream for
    /// its lifetime, so a slow-peer flood of half-open files cannot exhaust the
    /// blocking pool (one open file pins one blocking-pool thread).
    sessions: Arc<Semaphore>,
}

impl NativeFsHandler {
    /// Builds a handler that serves `config`, driving streams on `handle`.
    ///
    /// Concurrent streams are capped at [`MAX_CONCURRENT_SESSIONS`].
    #[must_use]
    pub fn new(config: NativeServeConfig, handle: Handle) -> Self {
        Self {
            config,
            handle,
            sessions: Arc::new(Semaphore::new(MAX_CONCURRENT_SESSIONS)),
        }
    }
}

impl fmt::Debug for NativeFsHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFsHandler")
            .field("guarded", &self.config.policy.is_some())
            .finish()
    }
}

impl ProtocolHandler for NativeFsHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        // Identity is read from the verified QUIC handshake (no 0-RTT), so
        // `remote_id` is the proven peer key before any frame is served. Bind it
        // once per connection.
        let peer = peer_id_for(connection.remote_id());
        // Resolve the per-connection root once. Default-deny: a peer with no
        // grant gets no streams served at all (the native EACCES).
        let Some(root) = self.config.resolve_root(peer) else {
            return Ok(());
        };
        let deadline = self.config.deadline;
        loop {
            let (send, recv) = match connection.accept_bi().await {
                Ok(streams) => streams,
                // No more streams: the peer closed the connection. Routine over
                // NAT/relay churn, not an error.
                Err(_) => return Ok(()),
            };
            // Admit at most MAX_CONCURRENT_SESSIONS live streams. Each holds one
            // blocking-pool thread for its lifetime; the owned permit moves into
            // the blocking task and releases when the stream ends. The semaphore
            // is never closed, so acquire only fails on shutdown.
            let Ok(permit) = Arc::clone(&self.sessions).acquire_owned().await else {
                return Ok(());
            };
            let root = Arc::clone(&root);
            let handle = self.handle.clone();
            // serve_one is synchronous: a one-shot op, or one open file running
            // the FileOp/FileReply loop (which may block on a never-EOF device).
            // Run each stream on a blocking-pool thread so a parked read never
            // pins a runtime worker and the BlockingDuplex can block_on safely.
            // Detached: every stream serves independently for the connection's
            // life, so a blocked read on one stream never stalls another — the
            // head-of-line property StreamingImportFs hand-discovers, made
            // structural by one-stream-per-open-file.
            tokio::task::spawn_blocking(move || {
                serve_one_stream(&root, send, recv, handle, deadline);
                // Permit held until the stream ends, then released here.
                drop(permit);
            });
        }
    }
}

/// Runs the synchronous native-wire server over one bridged stream.
///
/// The duplex is **asymmetric**: the server's read (the idle wait for the next
/// `FileOp` of a live, possibly never-EOF subscription) carries no deadline,
/// while the server's write of a `FileReply` carries the per-op deadline. This
/// is the same idle-read-vs-in-flight-write distinction the 9P handler makes
/// across its separate reader/writer halves.
fn serve_one_stream(
    root: &Arc<dyn FileSystem>,
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    handle: Handle,
    deadline: Option<Duration>,
) {
    let duplex = BlockingDuplex::with_deadlines(send, recv, handle, None, deadline);
    // serve_one's own `deadline` argument is advisory: the transport (the
    // asymmetric BlockingDuplex above) is what actually bounds the in-flight
    // write. Pass it through for symmetry with the documented contract.
    wanix_mesh_wire::serve_one(root, duplex, deadline);
}
