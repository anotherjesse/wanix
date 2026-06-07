//! The bulk data plane: an iroh-blobs-backed [`ContentStore`].
//!
//! [`IrohCasStore`] is the network half of venti. It wraps an iroh-blobs store
//! (memory or on-disk) and the node's [`iroh::Endpoint`], and implements the
//! synchronous [`wanix_cas::ContentStore`] trait through the same held-`Handle`
//! blocking bridge the 9P client uses — so the `#cas` device, capsule
//! materialization, and the offload hook all reach a peer-fetching store with no
//! async leakage.
//!
//! # API ground truth (validated against iroh-blobs 0.102.0, not the blueprint)
//!
//! The blueprint named provisional method shapes; the *real* resolved API is:
//!
//! - put: `store.blobs().add_bytes(bytes).await -> RequestResult<TagInfo>`,
//!   whose `.hash` is the BLAKE3 content address.
//! - local read: `store.blobs().get_bytes(hash).await -> ExportBaoResult<Bytes>`.
//! - presence: `store.blobs().has(hash).await -> irpc::Result<bool>`.
//! - fetch: `endpoint.connect(peer, iroh_blobs::ALPN).await` to a provider
//!   ticket, then `store.remote().fetch(connection, HashAndFormat::raw(hash))
//!   .await -> Result<Stats>`, then a local `get_bytes`. We open the connection
//!   from the full [`EndpointAddr`] ticket (not a bare-id downloader resolve) so
//!   first contact works on a LAN without relay/DNS (iroh #3713); see
//!   [`fetch_one`].
//!
//! iroh-blobs' `Hash` is itself BLAKE3, so it maps to [`wanix_fs::ContentHash`]
//! by raw 32 bytes with no rehashing. The blob plane shares one
//! [`iroh::Endpoint`]/[`Router`] with the 9P control plane via
//! [`iroh_blobs::ALPN`] (see [`blobs_protocol`]).
//!
//! # Trust
//!
//! iroh-blobs verifies blobs end-to-end against their BLAKE3 hash while
//! streaming, so a hostile peer cannot inject mismatched bytes. We still clamp
//! every returned blob to [`wanix_cas::MAX_BLOB_SIZE`] before handing it up,
//! because `get_bytes` loads the whole blob into memory and a hostile *ticket*
//! (large but valid) would otherwise OOM the importer.

use std::sync::Arc;

use iroh::{Endpoint, EndpointAddr};
use iroh_blobs::BlobsProtocol;
use iroh_blobs::HashAndFormat;
use iroh_blobs::api::Store as BlobStore;
use iroh_blobs::store::mem::MemStore;
use tokio::runtime::Handle;
use wanix_cas::{CasError, CasResult, ContentStore, MAX_BLOB_SIZE};
use wanix_fs::ContentHash;

/// Converts a Wanix [`ContentHash`] into an iroh-blobs [`iroh_blobs::Hash`].
///
/// Both are raw BLAKE3 digests, so this is a byte-for-byte rewrap.
#[must_use]
pub fn blob_hash(hash: &ContentHash) -> iroh_blobs::Hash {
    iroh_blobs::Hash::from(*hash.as_bytes())
}

/// Converts an iroh-blobs [`iroh_blobs::Hash`] into a Wanix [`ContentHash`].
#[must_use]
pub fn content_hash(hash: &iroh_blobs::Hash) -> ContentHash {
    ContentHash::from_bytes(*hash.as_bytes())
}

/// Builds the iroh-blobs [`BlobsProtocol`] handler to register under
/// [`iroh_blobs::ALPN`] on the node's shared [`Router`], so peers can fetch this
/// node's blobs over the same endpoint that serves 9P.
///
/// Pass the returned handler to `Router::builder(ep).accept(iroh_blobs::ALPN,
/// handler)` alongside the 9P handler: one endpoint, one identity, two planes.
#[must_use]
pub fn blobs_protocol(store: &IrohCasStore) -> BlobsProtocol {
    BlobsProtocol::new(store.blob_store(), None)
}

/// An iroh-blobs-backed content store reachable from synchronous Wanix code.
///
/// Cloning shares the underlying blob store, endpoint, runtime handle, and peer
/// set, so one node holds a single store that the device, capsules, and offload
/// hook all share.
#[derive(Clone)]
pub struct IrohCasStore {
    store: Arc<MemStore>,
    endpoint: Endpoint,
    handle: Handle,
    /// Candidate providers consulted, in order, when a blob is missing locally
    /// and must be fetched from the data plane.
    ///
    /// These are full [`EndpointAddr`] tickets (id + direct addresses + relay),
    /// not bare ids: the blueprint requires preferring a ticket with direct
    /// addresses for first contact, since a bare-id dial depends on relay/DNS
    /// discovery that is unavailable on a LAN and not immediately ready on the
    /// public network (iroh #3713). The store opens its own connection to a
    /// provider on [`iroh_blobs::ALPN`] and fetches over it.
    peers: Arc<Vec<EndpointAddr>>,
}

impl IrohCasStore {
    /// Creates an in-memory blob store over `endpoint`, driving async work on
    /// `handle`.
    ///
    /// `peers` are the candidate providers (full tickets) a `get` consults when
    /// the blob is not already local; pass the peers whose worlds/capsules this
    /// node imports. The memory store is the testable default; a persistent
    /// on-disk store is a follow-up (`FsStore::load`), which plugs into the same
    /// wrapper.
    #[must_use]
    pub fn memory(endpoint: Endpoint, handle: Handle, peers: Vec<EndpointAddr>) -> Self {
        // `MemStore::new` spawns its backing actor task, so it must be built with
        // the node's runtime entered. The actor then lives on that long-lived
        // runtime; we only borrow the context to spawn it. `block_on` is safe
        // here because this constructor runs on a non-runtime (caller) thread.
        let store = handle.block_on(async { MemStore::new() });
        Self {
            store: Arc::new(store),
            endpoint,
            handle,
            peers: Arc::new(peers),
        }
    }

    /// Returns the underlying iroh-blobs [`BlobStore`] for protocol registration.
    #[must_use]
    pub fn blob_store(&self) -> &BlobStore {
        &self.store
    }

    /// Returns the candidate provider tickets for missing-blob fetches.
    #[must_use]
    pub fn peers(&self) -> &[EndpointAddr] {
        &self.peers
    }
}

impl ContentStore for IrohCasStore {
    fn put(&self, bytes: &[u8]) -> CasResult<ContentHash> {
        if bytes.len() > MAX_BLOB_SIZE {
            return Err(CasError::TooLarge { len: bytes.len() });
        }
        let store = Arc::clone(&self.store);
        let owned = bytes.to_vec();
        let tag = self
            .handle
            .block_on(async move { store.blobs().add_bytes(owned).await })
            .map_err(|err| CasError::Backend(err.to_string()))?;
        Ok(content_hash(&tag.hash))
    }

    fn get(&self, hash: &ContentHash) -> CasResult<Vec<u8>> {
        let blob = blob_hash(hash);
        let store = Arc::clone(&self.store);
        let endpoint = self.endpoint.clone();
        let peers = Arc::clone(&self.peers);
        let bytes = self
            .handle
            .block_on(async move { fetch_blob(&store, &endpoint, &peers, blob).await })?;
        if bytes.len() > MAX_BLOB_SIZE {
            return Err(CasError::TooLarge { len: bytes.len() });
        }
        Ok(bytes.to_vec())
    }

    fn has(&self, hash: &ContentHash) -> CasResult<bool> {
        let blob = blob_hash(hash);
        let store = Arc::clone(&self.store);
        self.handle
            .block_on(async move { store.blobs().has(blob).await })
            .map_err(|err| CasError::Backend(err.to_string()))
    }
}

/// Reads a blob locally, fetching it from a provider peer first if it is absent.
///
/// iroh-blobs verifies the BLAKE3 hash end-to-end while streaming the fetch, so
/// the returned bytes are guaranteed to match `hash`. A missing-and-unfetchable
/// blob surfaces as [`CasError::NotFound`].
async fn fetch_blob(
    store: &MemStore,
    endpoint: &Endpoint,
    peers: &[EndpointAddr],
    hash: iroh_blobs::Hash,
) -> CasResult<bytes::Bytes> {
    if !store
        .blobs()
        .has(hash)
        .await
        .map_err(|err| CasError::Backend(err.to_string()))?
    {
        fetch_from_peers(store, endpoint, peers, hash).await?;
    }
    store
        .blobs()
        .get_bytes(hash)
        .await
        .map_err(|_| CasError::NotFound)
}

/// Tries each provider ticket in turn, opening a blobs-ALPN connection and
/// fetching `hash` into the local store. Succeeds on the first provider that
/// serves it; returns [`CasError::NotFound`] if none can.
///
/// Opening the connection from the full [`EndpointAddr`] (rather than letting the
/// downloader resolve a bare id) is what makes first contact work without
/// relay/DNS — the ticket carries the provider's direct addresses.
async fn fetch_from_peers(
    store: &MemStore,
    endpoint: &Endpoint,
    peers: &[EndpointAddr],
    hash: iroh_blobs::Hash,
) -> CasResult<()> {
    let content = HashAndFormat::raw(hash);
    let mut last_error = None;
    for peer in peers {
        match fetch_one(store, endpoint, peer.clone(), content).await {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or(CasError::NotFound))
}

/// Fetches `content` from a single provider over a fresh blobs-ALPN connection.
async fn fetch_one(
    store: &MemStore,
    endpoint: &Endpoint,
    peer: EndpointAddr,
    content: HashAndFormat,
) -> CasResult<()> {
    let connection = endpoint
        .connect(peer, iroh_blobs::ALPN)
        .await
        .map_err(|err| CasError::Backend(format!("connect failed: {err}")))?;
    store
        .remote()
        .fetch(connection, content)
        .await
        .map(|_stats| ())
        .map_err(|err| CasError::Backend(format!("fetch failed: {err}")))
}
