---
title: End-to-End Hash Verification
slug: concepts/end-to-end-hash-verification
pageType: concept
oneLiner: Every #cas read re-hashes bytes so a hostile peer cannot serve content under a wrong address; ingest is size-capped at write time.
audience: [developer, visionary]
tags: [cas, mesh, content-addressing, trust-boundary, shipped, caveat]
sourceRefs:
  - crates/wanix-cas/src/store.rs:113-132
  - crates/wanix-cas/src/local.rs:79-89
  - crates/wanix-cas/src/device/files.rs:90-125
  - crates/wanix-cas/src/device.rs:106-150
  - crates/wanix-cas/src/hash.rs:13-17
  - crates/wanix-mesh/src/cas.rs:30-37
  - crates/wanix-mesh/src/cas.rs:170-194
seeAlso:
  - devices/cas
  - concepts/content-addressed-data-plane
  - concepts/wanix-capsule
  - concepts/one-identity-two-planes
prerequisites:
  - devices/cas
  - concepts/content-addressed-data-plane
usedInFlows: []
honestLimits:
  - The content hash is BLAKE3; the same digest is the address and the integrity check, so trust rests on BLAKE3's collision resistance.
  - The mesh blob plane (IrohCasStore over iroh-blobs) is the network half and is upstream-experimental; the local LocalCasStore path is the stable one.
  - The size cap (MAX_BLOB_SIZE, 256 MiB) bounds a single hostile allocation but is not a per-peer or aggregate quota.
canonicalCaveatFor: []
---

# End-to-End Hash Verification

Every `#cas` read re-hashes the bytes it returns, so a hostile peer cannot serve content under a wrong address; ingest is size-capped at write time.

A content-addressed store names a blob by the hash of its bytes. That is not just a deduplication trick — it is the whole integrity story. If a blob's address is the BLAKE3 hash of its content, then any byte returned for that address can be checked by recomputing the hash. Wanix's `#cas` device runs that check on *every* read, local or remote, so a corrupted file on disk or a lying peer over the mesh is rejected with `HashMismatch` rather than handed up under an address it no longer matches.

## Show it: a tampered blob is rejected, never served

Ingest some bytes and `#cas` hands you back their address:

```sh
printf 'venti score' > '#cas/ingest'
cat '#cas/ingest'        # -> 9e2c... (the BLAKE3 content address)
cat '#cas/9e2c...'       # -> venti score
```

Now corrupt the stored blob behind the device's back — flip a byte on disk under the same address. The next read does not return the tampered bytes; it fails:

```sh
cat '#cas/9e2c...'       # error: hash mismatch: requested 9e2c..., got 1f80...
```

The store recomputed the hash of what it found, saw it no longer equals the requested address, and refused. That is the contract: bytes that do not match their address are not content, they are noise.

## verify_hash on every get — the address is the integrity check

The check lives in one function, `verify_hash`, used by every read path (`crates/wanix-cas/src/store.rs:113-132`):

```rust
pub fn verify_hash(bytes: &[u8], expected: &ContentHash) -> CasResult<()> {
    let actual = hash_bytes(bytes);
    if &actual == expected { Ok(()) }
    else { Err(CasError::HashMismatch { requested: *expected, actual }) }
}
```

`hash_bytes` is BLAKE3 (`crates/wanix-cas/src/hash.rs:13-17`), and the same digest is the content address — so there is no separate checksum to keep in sync. The on-disk `LocalCasStore::get` re-hashes on the way out before it returns a single byte (`crates/wanix-cas/src/local.rs:79-89`): "a same-user-corrupted file must not be served under a content address it no longer matches." The device then wraps the verified bytes in a snapshot reader captured once at open (`BytesReadFile`), so a concurrent change cannot tear an in-flight read (`crates/wanix-cas/src/device.rs:143-144`). Verification is not a flag you opt into; it is the only way bytes leave the store.

This is venti's idea, the archival store Plan 9 grew alongside Fossil: immutable blobs named by their score (hash), so equal bytes deduplicate to one entry and any reader can independently confirm what it got. Wanix keeps the score as a BLAKE3 `ContentHash`.

## Ingest is capped at write time, not just at put time

Reads are verified; writes are bounded. `#cas/ingest` buffers the bytes you write and `put`s them on close, publishing the resulting address for the next read of `#cas/ingest` — the "write-then-read-hash" handle. The subtle part is *where* the size cap fires. Because `#cas` is a plain `FileSystem`, it imports across the mesh, so a remote peer with write access to `#cas/ingest` could stream unbounded bytes into host memory before any close-time `put` cap ever ran. So the cap is enforced on every `write`, before the buffer grows (`crates/wanix-cas/src/device/files.rs:90-100`):

```rust
if self.buffer.len().saturating_add(buf.len()) > MAX_BLOB_SIZE {
    self.over_cap = true;
    return Err(FsError::NotSupported);
}
```

The saturating add means a pathological length cannot wrap and slip under the ceiling. And once a stream trips the cap the handle is *poisoned*: on drop, the over-cap flag suppresses the close-time `put` entirely (`crates/wanix-cas/src/device/files.rs:111-125`), so a truncated prefix of an over-cap stream never publishes as a blob and never masquerades as a successful ingest. You either get the whole blob's true address or nothing.

## Over the mesh: verified while streaming

The network half is `IrohCasStore` in `wanix-mesh`, the only async crate (`crates/wanix-mesh/src/cas.rs`). When a `get` finds a blob missing locally, it opens a connection to a provider peer over the iroh-blobs ALPN and fetches it. The fetch is not a raw byte transfer trusted on arrival: iroh-blobs verifies the blob end-to-end against its BLAKE3 hash *while streaming*, so a hostile peer cannot inject mismatched bytes (`crates/wanix-mesh/src/cas.rs:30-37`, `:170-194`). iroh-blobs' `Hash` is itself BLAKE3, so a Wanix `ContentHash` maps to it byte-for-byte with no rehashing — a locally-ingested blob and a peer-fetched blob share one address.

Two trust planes meet on one identity here (see [one identity, two planes](/concepts/one-identity-two-planes)): the 9P control plane and the blob data plane share the node's single `iroh::Endpoint`. The control plane offloads bulk bytes to the verified data plane above a size threshold instead of crawling read windows. Even after streaming verification, the mesh store re-clamps every returned blob to `MAX_BLOB_SIZE` before handing it up, because a hostile *ticket* — large but validly hashed — would otherwise OOM the importer.

## See also

- [#cas device](/devices/cas) — the device files: `<hash>` read, `ingest` write-then-read-hash, `have/<hash>` probe.
- [Content-addressed data plane](/concepts/content-addressed-data-plane) — why bulk bytes ride a separate, hash-named plane from the 9P control plane.
- [wanix capsule](/concepts/wanix-capsule) — freezing a world into a portable, CAS-backed `.wcap` whose blobs are verified the same way.
- [One identity, two planes](/concepts/one-identity-two-planes) — the single endpoint that carries both 9P and the blob plane.

## Status / honest limits

- **The hash is BLAKE3, and the address is the integrity check.** There is no separate signature or checksum; trust rests on BLAKE3's collision resistance. `hash_bytes` anchors to the real algorithm (`crates/wanix-cas/src/hash.rs:13-17`).
- **The mesh blob plane is upstream-experimental.** `IrohCasStore` rides iroh-blobs (validated against 0.102.0) and is the network half of venti; the local `LocalCasStore` path is the stable one. End-to-end streaming verification is iroh-blobs' guarantee, which Wanix relies on and re-clamps for size.
- **The size cap bounds one allocation, not a quota.** `MAX_BLOB_SIZE` (256 MiB) keeps a single hostile blob or ticket from exhausting host memory; it is not a per-peer or aggregate storage quota. Like the other exec/data devices, `#cas` write access over the mesh is a trust-boundary surface, not a sandbox for arbitrary untrusted peers.
