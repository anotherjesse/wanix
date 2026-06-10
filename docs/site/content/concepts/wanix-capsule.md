---
title: wanix capsule — CAS-Backed World Snapshots
slug: concepts/wanix-capsule
pageType: concept
oneLiner: Freeze a directory tree into content-addressed blobs whose deterministic manifest-blob hash is the capsule id — deduplicated, integrity-verified, and fetchable over the mesh blob plane.
audience: [developer, visionary]
tags: [cli, mesh, content-addressed, caveat, shipped]
sourceRefs:
  - crates/wanix-cas/src/capsule.rs:38-46
  - crates/wanix-cas/src/capsule.rs:138-189
  - crates/wanix-cas/src/capsule.rs:244-301
  - crates/wanix-cli/src/capsule.rs:138-169
  - crates/wanix-cas/src/store.rs:24-122
  - crates/wanix-mesh/src/cas.rs:147-194
  - docs/recipes/03-freeze-world-to-capsule.md
seeAlso:
  - devices/cas
  - concepts/end-to-end-hash-verification
  - concepts/content-addressed-data-plane
  - concepts/one-identity-two-planes
  - concepts/kv-smallest-database
  - use-cases/portable-worlds
  - recipes/03-freeze-world-to-capsule
prerequisites:
  - devices/cas
  - concepts/end-to-end-hash-verification
usedInFlows: []
honestLimits:
  - There is no .wcap archive file in the shipped CLI; a capsule is the manifest blob plus its referenced file blobs sitting in the CAS store.
  - Capsules freeze file content only — symlinks, permissions, and extended attributes are dropped; materialized files land mode 0o644.
  - Live #kv state is not in the world tree; freeze the values you want by copying them into the directory first.
  - A capsule id is itself a content address — resolving it over the mesh still needs the live peer relationship and iroh ticket, which the capsule does not carry.
---

# wanix capsule — CAS-Backed World Snapshots

Freeze a directory tree into content-addressed blobs whose deterministic manifest-blob hash is the capsule id — deduplicated, integrity-verified, and fetchable over the mesh blob plane.

A capsule is venti applied to a whole *world*. Take a directory the agent (or you) just built — qjs scripts, data, whatever — and freeze it onto the content-addressed plane so the exact same tree can be rebuilt, byte for byte, anywhere the capsule id is reachable. The id is the only durable thing you keep: share the id, share the world.

## Show it: save and load a world

A flow's first build step is `cargo build --package wanix-cli; alias wanix='./target/debug/wanix'`. Then build a tiny world and freeze it:

```sh
mkdir -p /tmp/world-A/scripts
echo 'hello, capsule' > /tmp/world-A/greeting.txt
cat > /tmp/world-A/scripts/hello.js <<'JS'
import * as std from "qjs:std";
std.out.puts(std.loadFile("greeting.txt"));
JS

export CAS=/tmp/cas-A
wanix capsule save /tmp/world-A --store "$CAS"
```

The visible result is a content hash and a copy-pasteable load command (`crates/wanix-cli/src/capsule.rs:138-148`):

```text
capsule 7f3a…<64 hex chars>… saved (2 files) from /tmp/world-A
load with: wanix capsule load 7f3a… <DIR>
```

That 64-hex string is the capsule id. Hand it to a different directory, a different user, or a different machine, and the world rebuilds:

```sh
wanix capsule load 7f3a… /tmp/world-B --store "$CAS"
# capsule 7f3a… restored (2 files, 38 bytes) to /tmp/world-B
diff -r /tmp/world-A /tmp/world-B   # no output → byte-identical
```

`load` materializes the tree and reports the file and byte counts it wrote (`crates/wanix-cli/src/capsule.rs:151-168`). No archive was produced; no archive was unpacked. The world moved as a hash.

## What the id actually names

The mechanism is small and worth seeing whole.

**Every file becomes a blob.** `Capsule::freeze` walks the tree, reads each regular file, and ingests its bytes into a `ContentStore`, keyed by content hash (`crates/wanix-cas/src/capsule.rs:244-258`). Two files with identical bytes — across this world or any other world frozen into the same store — collapse to one blob. Dedup is automatic because the address *is* the content.

**The manifest is itself a blob.** The per-file paths and hashes are collected into a `WorldManifest`, sorted by path, and serialized to a deterministic byte form: one `\n`-terminated `"<hex-hash> <path>"` line per entry (`crates/wanix-cas/src/capsule.rs:138-153`). Sorting makes the serialized form independent of insertion order, so the same input tree always produces the same manifest bytes.

**That manifest blob's hash is the capsule id.** `freeze` puts the manifest blob into the store and returns its hash as the id (`crates/wanix-cas/src/capsule.rs:256-257`). So the id is a content address like any other — but it transitively names the whole world: to receive a capsule you fetch the manifest blob by its id, parse it, then fetch each referenced file blob and write it out (`crates/wanix-cas/src/capsule.rs:271-275, 202-224`). The Plan 9 name for this is venti: a content-addressed archival store where the hash of a structure points at every leaf it depends on. A capsule is venti scoped to one directory tree.

Because the manifest blob is plain text, you can read it directly out of the store:

```sh
cat "$CAS/7f3a…"
# a91b…  greeting.txt
# 4c02…  scripts/hello.js
```

The store is one flat directory of blobs named by their lowercase-hex BLAKE3 (`docs/recipes/03-freeze-world-to-capsule.md`). There is no per-capsule subdirectory; blobs from every capsule pool together, which is exactly what lets dedup work across worlds.

## Materialize is the trust boundary

Loading a capsule means trusting whatever a manifest claims, so the load path is defensive by construction (`crates/wanix-cas/src/capsule.rs:13-24`). The same checks apply whether the id came from your own store or a hostile peer:

- **Path safety.** Every manifest path is re-validated through `NormalizedPath` — no `..`, no absolute path, no escaping component — and `safe_join` confines every write inside the target directory regardless of what the sender claimed (`crates/wanix-cas/src/capsule.rs:359-378`). A hostile manifest cannot write outside the target.
- **Fan-out cap.** Each entry forces a separate whole-blob fetch, so the manifest is capped at `MAX_MANIFEST_ENTRIES` (100k) entries (`crates/wanix-cas/src/capsule.rs:38, 207-209`). A manifest with millions of entries is rejected before any file is written.
- **Manifest-size cap.** The manifest blob is loaded whole to parse, so it is capped at `CAPSULE_MANIFEST_MAX_BYTES` (16 MiB) independently of the per-file cap (`crates/wanix-cas/src/capsule.rs:46, 164-166`).
- **Per-blob cap.** Every file blob is subject to the store's `MAX_BLOB_SIZE` (256 MiB) ceiling (`crates/wanix-cas/src/store.rs:24`), so one valid-but-huge blob cannot OOM the importer.
- **Re-hash on every get.** A `LocalCasStore::get` re-hashes the bytes it reads against the requested address via `verify_hash` before returning them (`crates/wanix-cas/src/store.rs:122`). A tampered file under `$CAS/<hex>` surfaces as a hash mismatch instead of being served under the wrong content address. See [end-to-end hash verification](/concepts/end-to-end-hash-verification).

One malicious id therefore yields an error, not an unsafe write.

## Over the mesh: blobs follow tickets

The `LocalCasStore` id is the local-store form of the mesh's `BlobTicket`. The same `ContentStore` trait has a network-fetching implementation, `IrohCasStore`, which resolves a missing blob by dialing a provider peer over `iroh_blobs::ALPN`, fetching it, then reading it locally (`crates/wanix-mesh/src/cas.rs:147-159`). iroh-blobs verifies each blob end-to-end against its BLAKE3 hash while streaming, and the wrapper still clamps every returned blob to `MAX_BLOB_SIZE` so a large-but-valid hostile ticket cannot OOM the importer (`crates/wanix-mesh/src/cas.rs:175-194`). So `capsule load <id>` resolves blobs through the iroh ticket a peer published, and the same local trust boundary — re-hash, size caps, `NormalizedPath` — still applies on materialize.

This is the bulk data plane of [one identity, two planes](/concepts/one-identity-two-planes): the 9P control plane carries small operations, and large content offloads to the content-addressed blob plane that the `#cas` device and capsules share. See [content-addressed data plane](/concepts/content-addressed-data-plane).

## See also

- [#cas device](/devices/cas) — the content store a capsule freezes into.
- [End-to-end hash verification](/concepts/end-to-end-hash-verification) — why a re-hash on every get is the integrity contract.
- [Content-addressed data plane](/concepts/content-addressed-data-plane) — the bulk-bytes side of the mesh.
- [One identity, two planes](/concepts/one-identity-two-planes) — how control and data planes share one endpoint.
- [#kv, the smallest database](/concepts/kv-smallest-database) — the live device state a capsule does *not* carry.
- [Portable worlds](/use-cases/portable-worlds) — the capsule use case end to end.
- [Recipe 03 — freeze a world to a capsule](/recipes/03-freeze-world-to-capsule) — the full round-trip you can paste into a shell.

## Status / honest limits

- **No `.wcap` archive file in the shipped CLI.** A capsule is the manifest blob plus its referenced file blobs, sitting in the CAS store (`docs/recipes/03-freeze-world-to-capsule.md`). To make a single transportable artifact, copy the manifest blob plus the blobs it names, or fetch them over the mesh.
- **Content only.** `collect_files` skips symlinks deliberately — a capsule freezes content, not link topology, since a dereferenced symlink could escape the world root (`crates/wanix-cas/src/capsule.rs:305-334`). Permissions, special files, and extended attributes are not represented; materialized files land mode `0o644`. Layer your own metadata file in if you need them.
- **Live `#kv` state is not in the tree.** `#kv` is a service device, not a directory under the world root, and it is in-memory — its state lives only as long as the serve process. To freeze KV values, copy the ones you care about into the world directory before `save`.
- **A capsule id is not a peer.** The id is itself a content address. Resolving it over the network still needs the live peer relationship and the iroh ticket, which are runtime state, not part of the capsule. Live peers, leases, ephemeral fds, open `#term`/`#task` handles, and process identity are all explicitly out of scope.
