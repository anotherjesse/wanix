---
title: Portable Worlds via Capsules
slug: use-cases/portable-worlds
pageType: use-case
oneLiner: wanix-rust capsule save freezes a whole world — the directory an agent built — into a CAS-backed, BLAKE3-verified, deduplicated set of blobs, and hand someone one hash and they materialize the entire world, verifying every blob.
audience: [visionary, developer]
tags: [shipped, cli, mesh, content-addressed, caveat]
sourceRefs:
  - docs/recipes/03-freeze-world-to-capsule.md
  - crates/wanix-cas/src/capsule.rs
  - crates/wanix-cas/src/capsule.rs:144-189
  - crates/wanix-cas/src/capsule.rs:244-301
  - crates/wanix-cas/src/capsule.rs:305-334
  - docs/mesh-the-missing-half-of-9p.md:1559-1672
seeAlso:
  - concepts/wanix-capsule
  - devices/cas
  - concepts/end-to-end-hash-verification
  - concepts/content-addressed-data-plane
  - concepts/one-identity-two-planes
  - concepts/quickjs-snapshots-are-vm-images
  - recipes/03-freeze-world-to-capsule
prerequisites:
  - concepts/wanix-capsule
  - concepts/end-to-end-hash-verification
usedInFlows: []
honestLimits:
  - There is no .wcap archive file in the shipped CLI — a capsule is the manifest blob plus the file blobs it references, sitting in the CAS store.
  - A capsule freezes file content only — symlinks, permissions, extended attributes, and special files are not represented (materialized files land mode 0o644).
  - Live #kv state is not in the world tree; freeze it by copying the values you want into the directory before saving.
  - Live mesh peers, iroh tickets, and ephemeral fds (open #term/#task handles, in-flight 9P fids) are runtime state, not part of the capsule.
canonicalCaveatFor: []
---

# Portable Worlds via Capsules

`wanix-rust capsule save` freezes a whole world — the directory an agent built — into a CAS-backed, BLAKE3-verified, deduplicated set of blobs; hand someone one hash and they materialize the entire world, verifying every blob.

**What & why.** An agent just spent an afternoon building a working directory: qjs scripts, a generated dataset, some state. How do you ship that *exact* world to a teammate, a cloud box, or your future self — and prove on the other end that what arrived is byte-for-byte what left? The usual answers are a tarball you have to trust, a Docker image you have to rebuild, or a git remote you have to host. Wanix offers a smaller primitive: freeze the directory onto a content-addressed store and you get back one short hash. That hash *is* the world. Anyone who can reach the blobs reconstructs the tree, and every blob is re-hashed against its content address before a single byte hits disk. This is venti — Plan 9's archival store — applied to a whole world.

## The outcome: ship a world by one hash

Build a small world and freeze it. Recipe 03 walks the full thing; here is the spine ([docs/recipes/03-freeze-world-to-capsule.md](/recipes/03-freeze-world-to-capsule)):

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

mkdir -p /tmp/world-A/scripts /tmp/world-A/state
printf 'hello, capsule\n' > /tmp/world-A/state/greeting.txt
cat > /tmp/world-A/scripts/hello.js <<'JS'
import * as std from "qjs:std";
std.out.puts(std.loadFile("state/greeting.txt"));
JS

export CAS=/tmp/cas-A
wanix-rust capsule save /tmp/world-A --store "$CAS"
# capsule 7f3a…<64 hex>… saved (2 files) from /tmp/world-A
# load with: wanix capsule load 7f3a… <DIR>
```

That `7f3a…` is the only durable thing you need. On any host where the store contains (or can fetch) the referenced blobs:

```sh
wanix-rust capsule load 7f3a… /tmp/world-B --store "$CAS"
diff -r /tmp/world-A /tmp/world-B   # no output -> byte-identical
```

The effect — one token reconstructs an entire directory tree — is the **capsule**. The id you shared is the BLAKE3 hash of a manifest, and the manifest names every file by *its* hash. Nothing about the bytes is taken on faith.

## venti for the mesh: file blobs plus a manifest blob whose hash is the id

`Capsule::freeze` walks the tree, ingests **each regular file as one blob**, then builds a deterministic sorted `WorldManifest` and ingests *that* as a blob too — its hash is the capsule id (`crates/wanix-cas/src/capsule.rs:244-258`). The manifest's serialized form is plain text: one `\n`-terminated `"<hex-hash> <path>"` line per entry, entries sorted by path (`WorldManifest::to_blob`, `crates/wanix-cas/src/capsule.rs:144-153`). You can read it:

```sh
cat "$CAS/7f3a…"
# a91b…b7  scripts/hello.js
# 4c02…d3  state/greeting.txt
```

Because entries are sorted, the byte form is independent of insertion order, so the same input tree freezes to the same id every time, on every host. That stability is load-bearing: the manifest's hash *is* the share token, so the serialization must never drift. The split here is exactly Plan 9's fossil/venti split — the 9P control plane keeps naming the live tree; the content-addressed plane carries the frozen-and-bulk ([docs/mesh-the-missing-half-of-9p.md:1639-1672](/concepts/content-addressed-data-plane)).

## Dedup across worlds, integrity-verified on every get

A `LocalCasStore` is one flat directory of blobs, each filename the lowercase-hex BLAKE3 of its bytes — no per-capsule subdirectory. Blobs from every capsule pool together, which is the whole point: identical bytes land on the same name, so two worlds that share a `lib.js` store it once. Dedup is not a feature bolted on; it is what content addressing *is*.

Integrity is the other half. Every store `get` re-hashes the bytes before returning them, so a blob that was corrupted or substituted under `$CAS/<hex>` is rejected with a hash mismatch rather than served under the wrong content address (`crates/wanix-cas/src/capsule.rs:213` materializes through that verifying `get`). Materialization is the trust boundary for an *incoming* capsule, so it is defensive by construction: the manifest is capped at `MAX_MANIFEST_ENTRIES` (100k) and `CAPSULE_MANIFEST_MAX_BYTES` (16 MiB) before any file write, every blob is bounded by the store's `MAX_BLOB_SIZE`, and every path is re-validated through `NormalizedPath` and `safe_join`, so a hostile manifest cannot escape the target directory or OOM the host (`crates/wanix-cas/src/capsule.rs:163-224`, `:359-378`). See [end-to-end hash verification](/concepts/end-to-end-hash-verification).

## Fetchable over the mesh blob plane

Locally the capsule id is just the id. On the mesh, the same `ContentStore` trait is backed by `IrohCasStore` — `iroh-blobs` on a second ALPN registered on the *same* router as 9P, so one endpoint and one identity carry two planes ([docs/mesh-the-missing-half-of-9p.md:1630-1637](/concepts/one-identity-two-planes)). A bare score told you *what* a block was but not *where*; iroh's `BlobTicket` carries the hash *and* the provider's direct addresses, so the id becomes self-locating. Hand someone the ticket and they have both the name and the route; `capsule load <id>` resolves blobs through the peer's published ticket, and the same local trust boundary — re-hash, size caps, `NormalizedPath` — still applies on materialize, so a hostile remote cannot escape the target tree.

This is where capsules and [the `#cas` device](/devices/cas) meet the rest of Wanix: `#cas` is a plain `FileSystem`, so it imports across the mesh for free, and a capsule is just blobs in that store. Shipping a world is handing over one hash.

## Runnable recipe

Walk the whole round-trip — build a world, freeze it, inspect the CAS structure, load it elsewhere, and prove tamper-detection — in [Recipe 03 — Freeze a Wanix world to a portable capsule](/recipes/03-freeze-world-to-capsule).

## See also

- Concepts: [the wanix capsule](/concepts/wanix-capsule) · [end-to-end hash verification](/concepts/end-to-end-hash-verification) · [the content-addressed data plane](/concepts/content-addressed-data-plane) · [one identity, two planes](/concepts/one-identity-two-planes)
- Devices: [the `#cas` device](/devices/cas) · [the `#kv` device](/devices/kv)
- Adjacent mechanism: [QuickJS snapshots are VM images](/concepts/quickjs-snapshots-are-vm-images) — the way to carry a *running* task, since a capsule carries a tree, not a process.
- Use case: [your personal compute mesh](/use-cases/personal-compute-mesh) — where the peers and tickets that resolve a capsule over the network come from.

## Status / honest limits

Capsule save/load is shipped and runnable today. Be precise about the edges:

- **No `.wcap` archive file in the shipped CLI.** A capsule is the manifest blob plus the file blobs it references, sitting in the CAS store (`docs/recipes/03-freeze-world-to-capsule.md:16-20`). For a single transportable artifact, copy the manifest blob at `$CAS/<capsule-id>` plus its referenced blobs, or resolve over the mesh blob plane.
- **Content only, not metadata.** Symlinks are intentionally skipped during freeze (`collect_files`, `crates/wanix-cas/src/capsule.rs:318-322`) because a capsule freezes content, not link topology, and a dereferenced symlink could escape the world root. Permissions, extended attributes, and special files are not represented; materialized files land mode `0o644`. If you need those, layer your own metadata file into the world.
- **Live `#kv` state is not in the tree.** `#kv` is a service device, not a directory under the world root, and it is in-memory — its state lives only as long as the serve process. Freeze the keys you care about by copying their values into the world tree before `capsule save`; a capsule is one way to persist them past a process exit.
- **Runtime state is not portable.** Live mesh peers and iroh tickets are the relationship needed to *resolve* an id over the network, not part of it; open `#term`/`#task` handles and in-flight 9P fids vanish when the originating process exits; process identity, environment, and cwd are the caller's to set on relaunch. To carry a *running* task rather than a tree, use the qjs snapshot/resume path instead.
