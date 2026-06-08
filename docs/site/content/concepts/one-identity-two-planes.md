---
title: One Identity, Two Planes (9P Control + iroh-blobs Data)
slug: concepts/one-identity-two-planes
pageType: concept
oneLiner: The 9P control plane and the BLAKE3 bulk data plane (iroh-blobs) accept on the same identity-bound endpoint, so a peer reaches both over one QUIC path.
audience: [developer]
tags: [mesh, cas, data-plane, exploratory, caveat]
sourceRefs:
  - crates/wanix-mesh/src/node.rs:146-228
  - crates/wanix-mesh/src/cas.rs:1-237
  - crates/wanix-fs/src/traits.rs:181-205
  - crates/wanix-fs/src/lib.rs:22-31
  - crates/wanix-cas/src/store.rs:24
  - crates/wanix-cli/src/mesh/serve.rs:165
seeAlso:
  - concepts/content-addressed-data-plane
  - devices/cas
  - concepts/wanix-capsule
  - concepts/9p-over-iroh-quic
prerequisites:
  - concepts/9p-over-iroh-quic
  - concepts/content-addressed-data-plane
usedInFlows: []
honestLimits:
  - "FileSystem::content_hash defaults to Ok(None); only a CAS-backed filesystem overrides it, so most files still read inline over 9P."
  - "The default mesh serve registers the 9P ALPN only (node.serve); the blob plane (serve_with_blobs) is a separate, opt-in registration."
  - "The blob data plane is experimental; the offload hook is designed but not fully wired into the default serve path."
---

# One Identity, Two Planes (9P Control + iroh-blobs Data)

The 9P control plane and the BLAKE3 bulk data plane (iroh-blobs) accept on the same identity-bound endpoint, so a peer reaches both over one QUIC path.

A 9P `Tread` window is the wrong tool for a 256 MiB rootfs image. Crawling a frozen world through `msize`-bounded reads is slow, and every byte rides the control plane that is also doing walks, stats, and mutations. Wanix's answer is not a second connection to a second service: it is a second *ALPN* on the *same* endpoint. One node, one ed25519 identity, one QUIC path — and two protocols multiplexed over it. The control plane moves structure; the data plane moves bulk bytes, BLAKE3-verified. This page is how that split is built and where it stops.

## The one-liner and the two ALPNs

A mesh node binds exactly one `iroh::Endpoint` from its persisted secret key (`crates/wanix-mesh/src/node.rs`). Everything rides that endpoint. What distinguishes a filesystem walk from a blob fetch is the ALPN — the application protocol label QUIC negotiates at connect time:

- **`wanix/fs/1`** — the control plane between two Wanix nodes: the [native FileSystem-over-iroh wire](/concepts/missing-half-of-9p) (`WANIX_FS_ALPN`). Walk, stat, read, write, mutation as `postcard` frames.
- **`wanix/9p/1`** — the same control plane for a *foreign* peer that speaks only 9P (`WANIX_9P_ALPN`).
- **iroh-blobs' own ALPN** (`BLOBS_ALPN`, re-exported from `iroh_blobs::ALPN`) — the data plane. BLAKE3 bulk blobs.

This page's split is control plane vs **data** plane — structure vs bulk bytes — and that distinction is unchanged whichever control-plane ALPN a peer negotiates. The worked example below uses the 9P control plane because the surrounding code (`p9_handler`) predates the native wire; a `serve_native` node accepts `WANIX_FS_ALPN` the same way.

The node serves both from one `Router` (`crates/wanix-mesh/src/node.rs:218`):

```rust
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
```

`Router::spawn` advertises both ALPNs from its accepted handlers, so a dialing peer can negotiate either over the one identity-bound path. The endpoint's own doc comment names the invariant: "the 9P `Router`, the blob downloader, and any directly opened bi-stream all ride this one identity-bound endpoint, which is what lets the data plane and control plane share a peer" (`crates/wanix-mesh/src/node.rs:146`).

## Two planes, one peer

The control plane you already know from [9P over iroh QUIC](/concepts/9p-over-iroh-quic): `RemoteFs` mounts a peer's namespace, and every walk/read/stat is a 9P frame on `wanix/9p/1`. The data plane is `IrohCasStore` — the network half of venti (`crates/wanix-mesh/src/cas.rs`). It wraps an iroh-blobs store and the *same* endpoint, and implements the synchronous `wanix_cas::ContentStore` trait through the held-`Handle` blocking bridge, so the `#cas` device, capsule materialization, and the offload hook all reach a peer-fetching store with no async leaking up.

A node builds its store over its own endpoint and a list of candidate provider tickets (`crates/wanix-mesh/src/node.rs:166`):

```rust
let cas = node.cas_store(vec![peer_ticket]);
node.serve_with_blobs(config, &cas);
```

`cas_store` takes full `EndpointAddr` tickets (id + direct addresses + relay), not bare ids. That is deliberate: opening the blob connection from a full ticket makes first contact work on a LAN without relay or DNS, sidestepping iroh #3713 (`crates/wanix-mesh/src/cas.rs:88-93`).

## IrohCasStore fetches from provider tickets on the shared endpoint

The interesting verb is `get`. When a hash is missing locally, the store fetches it from a provider over the blobs ALPN on the same endpoint (`crates/wanix-mesh/src/cas.rs:221`):

```rust
async fn fetch_one(store, endpoint, peer, content) -> CasResult<()> {
    let connection = endpoint.connect(peer, iroh_blobs::ALPN).await?;
    store.remote().fetch(connection, content).await.map(|_| ())
}
```

`fetch_from_peers` tries each provider ticket in turn and succeeds on the first that serves the blob; if none can, it surfaces `CasError::NotFound`. Because the blob store shares the node's endpoint, no second socket, no second identity, and no second NAT-traversal story is involved — the blob fetch is just another connection on the path the 9P session already uses.

## BLAKE3-verified end to end, clamped to MAX_BLOB_SIZE

The data plane is content-addressed, and that is its security model. iroh-blobs' `Hash` *is* BLAKE3, so it maps to `wanix_fs::ContentHash` by raw 32 bytes with no rehashing (`crates/wanix-mesh/src/cas.rs:53-61`). iroh-blobs verifies every blob end-to-end against its BLAKE3 hash *while streaming the fetch*, so a hostile peer cannot inject mismatched bytes — bad bytes fail verification before they reach you. See [end-to-end hash verification](/concepts/end-to-end-hash-verification).

Verification stops a *content* attack but not a *resource* attack: a valid-but-enormous ticket would OOM the importer, because `get_bytes` loads the whole blob into memory. So the store clamps every returned blob to `MAX_BLOB_SIZE` (256 MiB, `crates/wanix-cas/src/store.rs:24`) on both `put` and `get`, refusing oversized blobs with `CasError::TooLarge` (`crates/wanix-mesh/src/cas.rs:134-159`).

## Why content_hash is the offload hook

The bridge between the two planes lives in the `FileSystem` trait itself. `content_hash` is the *only* hook by which the 9P control plane offloads bulk bytes to the data plane (`crates/wanix-fs/src/traits.rs:181-205`):

```rust
fn content_hash(&self, _path: &NormalizedPath) -> FsResult<Option<ContentHash>> {
    Ok(None)
}
```

A CAS-aware client that learns a hash for a large file fetches the BLAKE3-verified blob peer-to-peer instead of crawling the bytes through the `msize`-bounded `Tread` window. The threshold is `CONTENT_HASH_OFFLOAD_THRESHOLD`, 256 KiB (`crates/wanix-fs/src/lib.rs:22-31`): below it, an inline read over a couple of `msize` windows is cheaper than a blob round-trip, so offload only pays off for genuinely large bulk content — frozen worlds, rootfs images, module inputs. A CAS-backed filesystem overrides `content_hash` to return the current hash, and must return `None` while a file is open for write so a client never fetches a torn snapshot. The hash travels out of band (a synthetic xattr-style file or a versioned protocol field), never appended to a fixed-shape 9P response, because the `Rgetattr` decoder rejects trailing bytes. This is the same narrow door [capsules](/concepts/wanix-capsule) ride to dedup and verify a frozen world across the mesh.

## See also

- [Content-addressed data plane](/concepts/content-addressed-data-plane) — venti, the `ContentStore` trait, and why hashes are addresses.
- [#cas device](/devices/cas) — the file-shaped front end: `<hash>` read, `ingest`, `have/<hash>`.
- [9P over iroh QUIC](/concepts/9p-over-iroh-quic) — the control plane this page pairs with.
- [Wanix capsule](/concepts/wanix-capsule) — a frozen, CAS-backed `.wcap` world that travels the data plane.
- [End-to-end hash verification](/concepts/end-to-end-hash-verification) — what BLAKE3-while-streaming buys you.

## Status / honest limits

- **`content_hash` defaults to `Ok(None)`.** Most filesystems have no blob backing; only a CAS-backed filesystem overrides the hook (`crates/wanix-fs/src/traits.rs:203`). For ordinary files, "two planes" collapses back to one: the bytes read inline over 9P, exactly as before.
- **The default mesh serve registers the 9P ALPN only.** The shipped `wanix-rust mesh serve` path calls `node.serve(config)` (`crates/wanix-cli/src/mesh/serve.rs:165`), which advertises `wanix/9p/1` alone. The blob plane is `serve_with_blobs` — a separate, opt-in registration. The two-ALPN router exists in `wanix-mesh`; wiring it into the default serve is not done.
- **The blob data plane is experimental.** The `IrohCasStore` fetch path, the offload threshold, and the `content_hash` door are designed and unit-exercised, but the end-to-end "9P session transparently offloads a large read to a peer's blob plane" flow is not fully wired in the default serve. Treat the data plane as a working substrate for capsules and `#cas`, not yet as an automatic large-file accelerator on every mount.
- **The store is in-memory by default.** `IrohCasStore::memory` is the testable default; a persistent on-disk store (`FsStore::load`) plugs into the same wrapper but is a follow-up (`crates/wanix-mesh/src/cas.rs:106`).
