---
title: "#cas — Content-Addressed Store (venti)"
slug: devices/cas
pageType: reference
oneLiner: "#cas/<hash> reads a verified blob, #cas/ingest is write-then-read-hash, and #cas/have/<hash> returns 1\\n or 0\\n."
audience: [developer]
tags: [device, cas, mesh, content-addressed, shipped, caveat]
sourceRefs:
  - crates/wanix-cas/src/device.rs:1-180
  - crates/wanix-cas/src/device/files.rs:47-125
  - crates/wanix-cas/src/store.rs:16-132
  - crates/wanix-cas/src/local.rs:58-94
  - crates/wanix-fs/src/content_hash.rs:56-77
seeAlso:
  - concepts/end-to-end-hash-verification
  - concepts/content-addressed-data-plane
  - concepts/wanix-capsule
  - concepts/one-identity-two-planes
  - devices/kv
prerequisites:
  - concepts/service-devices
  - concepts/content-addressed-data-plane
usedInFlows:
  - {flow: recipes/03-freeze-world-to-capsule, step: 2}
honestLimits:
  - "Ingest is capped at MAX_BLOB_SIZE = 256 MiB; a write past the cap is rejected with NotSupported and poisons the handle so the partial blob is never committed."
  - "Reading #cas/ingest before any blob has been ingested on the same device returns an empty string, not an error."
  - "The keyspace is not enumerable: the root lists only ingest and have/, and have/ lists nothing — you must already know a hash to read or probe it."
  - "LocalCasStore is owner-private and on-disk; it is durable, but the served #cas over the mesh is only as available as the serving node."
---

# #cas — Content-Addressed Store (venti)

`#cas/<hash>` reads a verified blob, `#cas/ingest` is write-then-read-hash, and `#cas/have/<hash>` returns `1\n` or `0\n`.

`#cas` is Wanix's venti: a store where the name of a blob *is* its content. You never choose a key — you write bytes, and the device hands you back the BLAKE3 hex digest those bytes hashed to. Read that digest and you get the exact same bytes, or an error; the store re-hashes on the way out so it can never serve the wrong bytes under a name. Equal bytes collapse to one entry automatically, so the store dedups for free. It is just a `FileSystem` like every other service device (`crates/wanix-cas/src/device.rs:1-18`), which means it imports across the mesh with no device-specific networking — a peer's store is reachable at `/n/<peer>/#cas/...` exactly the way the local one is reachable at `#cas/...`. This page is the file contract: what the three paths do, what they refuse, and where the bytes actually live.

## Three paths, and nothing else

`#cas` has a deliberately small surface. The path parser (`crates/wanix-cas/src/device.rs:84-102`) recognizes exactly four shapes — the root, `ingest`, `have`, `have/<hash>`, and a bare `<hash>` — and rejects everything else.

### `#cas/<hash>` — read a verified blob

```sh
wanix serve --wanix-services
# in a task or 9P client:
cat '#cas/4a8a08...'      # 64 lowercase-hex chars
```

Reading a bare hash returns the blob's bytes. The hash component is parsed by `ContentHash::from_hex` *before* any store lookup (`crates/wanix-cas/src/device.rs:101`), so a malformed or hostile path — wrong length, uppercase, non-hex — is rejected as `InvalidPath` at parse time and never reaches the backend (`crates/wanix-fs/src/content_hash.rs:63-77`). The path is read-only: opening a blob with `write`, `create`, or `truncate` returns `PermissionDenied` (`crates/wanix-cas/src/device.rs:140-142`). `stat` on a blob reports its length (`crates/wanix-cas/src/device.rs:159`); the file mode is `0o444`. Content addresses are immutable names, so the read end is immutable too.

### `#cas/ingest` — write-then-read-hash

This is the only writable file, and its trick is the contract: write bytes, close the handle, then *read the same file* to learn the content address of what you just stored.

```sh
printf 'hello venti' > '#cas/ingest'
cat '#cas/ingest'     # -> a8c2... (the hex hash of "hello venti")
```

Opening `ingest` for write returns an `IngestFile` that buffers bytes in memory. On close — Rust's `Drop` — the buffer is `put` into the store and the resulting lowercase-hex address is published into a shared `last_ingest` slot on the device (`crates/wanix-cas/src/device/files.rs:111-124`). Opening `ingest` *without* write returns whatever address that slot holds (`crates/wanix-cas/src/device.rs:131-137`). So the writer learns the address of its own blob with no out-of-band channel — the round trip is the demo (`#cas write-then-read-hash round trip`). The slot is per-device and reflects only the most recent ingest, not a history.

### `#cas/have/<hash>` — presence probe

```sh
cat '#cas/have/a8c2...'    # 1\n if present locally, 0\n if not
```

`have/<hash>` is a cheap existence check that does *not* load or verify the bytes (`crates/wanix-cas/src/store.rs:102-110`). It reads `1\n` when the blob is present locally and `0\n` when it is not (`crates/wanix-cas/src/device.rs:110-117`). The hash is validated the same way as a blob read, so an ill-formed probe path is rejected before the lookup.

## The keyspace is not enumerable, by design

A content-addressed store is looked up by hash, never browsed. `read_dir` reflects that: the root advertises only `ingest` and `have/`, and listing `have/` returns the empty set (`crates/wanix-cas/src/device.rs:167-179`). There is no path that lists stored blobs. To read or probe a blob you must already hold its hash — which you got either by ingesting it, by being told it (a capsule manifest, a plumb message, a peer), or by computing it from the bytes yourself. This is not an omission; it is what makes the keyspace a flat, unbounded namespace of immutable names that need no directory.

## Every read re-hashes; dedup is automatic

The store contract behind the device is `ContentStore` (`crates/wanix-cas/src/store.rs:81-111`): `put(bytes) -> hash`, `get(hash) -> bytes`, `has(hash) -> bool`. `put` returns the hash of the bytes, so two `put`s of identical bytes return the same address and the second write is a no-op against an already-present blob (`crates/wanix-cas/src/local.rs:66-69`). `get` re-hashes the bytes it loaded and refuses to return them under a name they no longer match — `HashMismatch` (`crates/wanix-cas/src/store.rs:113-132`, `crates/wanix-cas/src/local.rs:85-88`). That guarantee holds against a corrupted local file *and* against a hostile remote peer over the mesh: the bytes are checked where they are consumed, not where they were produced. This is the end-to-end verification property the data plane leans on; the concept page [end-to-end hash verification](/concepts/end-to-end-hash-verification) covers why the check lives at the read, not the write.

## Where the bytes live

The shipped local backing is `LocalCasStore` (`crates/wanix-cas/src/local.rs:58-94`): one file per blob, named by its lowercase-hex hash, in an owner-private directory under `$WANIX_CAS_DIR` or a per-user cache root. Writes are atomic (temp-file + rename) and reads are fd-verified through `wanix-module-cache`'s `AuditedBlobDir`, so a peer-user cannot pre-seed a blob the victim later trusts. The async, network-fetching `IrohCasStore` lives in `wanix-mesh` and is reached through the same blocking bridge as the 9P client, keeping `wanix-cas` synchronous and engine-free.

## #cas backs capsules

The content-addressed store is also the storage layer under [`wanix capsule`](/concepts/wanix-capsule): a `.wcap` freezes a Wanix world by writing each blob into a `ContentStore` and recording the manifest of hashes. Loading a capsule elsewhere re-materializes the world from those addresses, with the same `MAX_BLOB_SIZE` clamp guarding materialization. So `#cas` is both an operable device and the persistence substrate that lets `#kv`'s in-memory state and a whole namespace survive a process restart — see [content-addressed data plane](/concepts/content-addressed-data-plane) and [one identity, two planes](/concepts/one-identity-two-planes) for how the data plane sits beside the control plane.

## See also

- [End-to-end hash verification](/concepts/end-to-end-hash-verification) — why the re-hash lives at the read.
- [Content-addressed data plane](/concepts/content-addressed-data-plane) — the store as Wanix's bulk-bytes plane.
- [wanix capsule](/concepts/wanix-capsule) — freezing a world into a CAS-backed `.wcap`.
- [One identity, two planes](/concepts/one-identity-two-planes) — control plane vs. data plane.
- [#kv](/devices/kv) — the mutable state device whose in-memory state you persist by freezing to a capsule.
- [Freeze a world to a capsule](/recipes/03-freeze-world-to-capsule) — the recipe that exercises `#cas` end to end.

## Status / honest limits

- **Ingest is capped at 256 MiB.** `MAX_BLOB_SIZE` is enforced *at write time*, not just at the close-time `put` (`crates/wanix-cas/src/device/files.rs:90-100`): a remote peer with write access to `#cas/ingest` could otherwise stream unbounded bytes into host memory. Crossing the cap returns `NotSupported` and poisons the handle (`over_cap`), so the close-time `put` is skipped and a truncated prefix never publishes as a blob (`crates/wanix-cas/src/device/files.rs:111-124`).
- **Reading `#cas/ingest` before any ingest returns an empty string,** not an error — the `last_ingest` slot is simply unset (`crates/wanix-cas/src/device.rs:131-137`).
- **The keyspace is not enumerable.** There is no listing of stored blobs; you must already hold a hash to read or probe one (`crates/wanix-cas/src/device.rs:167-179`).
- **Durability is the serving node's.** `LocalCasStore` is on-disk and owner-private, so blobs survive a restart locally — but a `#cas` imported over the mesh is only as available as the node that serves it. To make a world portable, freeze it to a capsule.
