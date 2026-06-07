---
title: Glossary
slug: find/glossary
pageType: glossary
oneLiner: A-Z one-line definitions for every term and typed primitive an expert searches by exact name.
audience: [newcomer, developer, visionary]
tags: [find, reference, mesh, cli, shipped, local-trust-only, caveat]
sourceRefs:
  - AGENTS.md
  - crates/wanix-protocol/src/lib.rs
  - crates/wanix-id/src/policy.rs:1-45
  - crates/wanix-id/src/grant.rs:1-35
  - crates/wanix-vfs/src/subtree.rs:8-75
  - crates/wanix-vfs/src/binding.rs:7-24
  - crates/wanix-fs/src/path.rs:7-63
  - crates/wanix-fs/src/error.rs:9-42
  - crates/wanix-fs/src/content_hash.rs:21-35
  - crates/wanix-id/src/peer.rs:3-20
  - crates/wanix-id/src/identity.rs:5-50
  - crates/wanix-mesh/src/lib.rs:68
  - crates/wanix-mesh/src/cpu.rs:34-39
  - crates/wanix-mesh/src/identity.rs:23-29
  - crates/wanix-9p/src/lib.rs:53-135
  - crates/wanix-9p/src/dispatch.rs:45
  - crates/wanix-9p/src/error.rs:15
seeAlso: [the-9p-contract, attach-policy, normalizedpath, capability-is-a-bind, find/concepts, find/tags]
prerequisites: []
usedInFlows: []
honestLimits:
  - "Glossary entries link to the canonical page for each term; the one-line definition is a pointer, not the full contract."
  - "Tauth is answered ENOSYS: there is no 9P auth handshake; identity is the QUIC/ed25519 layer below."
  - "PeerId-keyed per-peer namespaces are designed; the shipped CLI mount binds a single /n/remote slot."
canonicalCaveatFor: []
---

# Glossary

A-Z one-line definitions for every term and typed primitive an expert searches by exact name.

This page is for the reader who already knows what they want and is typing the exact word into the search box: `NormalizedPath`, `msize`, `Tauth`, `SubtreeFs`. Each entry is one sentence of plain definition, a deep link to the page that owns the full contract, and the crate source that backs the claim. The entries are grouped Plan 9 vocabulary first, then the Rust typed primitives, because the second set is where an expert most often wants to confirm "is this an `i32` flag or a real newtype, and where is it enforced?" When a one-liner and the linked page disagree, the linked page (and the cited source) wins. (`AGENTS.md` is the canonical capability map this glossary indexes.)

## Plan 9 and Wanix vocabulary

- **bind** — Splice a filesystem into the namespace at a path; in Wanix a *capability* is literally a bind, not an ACL entry. See [capability is a bind](/concepts/capability-is-a-bind) and [namespace binding](/concepts/namespace-binding).
- **device** (`#name`) — A service exposed as a filesystem tree under a `#` name (`#kv`, `#pipe`, `#cas`, `#agent`, `#cpu`, `#task`, `#term`); every device is a plain `FileSystem`, so it imports across the mesh for free. See [service devices](/concepts/service-devices) and [devices index](/devices/index).
- **everything is a file** — The organizing rule: state, compute, agents, and remote machines are all files you open, read, and write. See [everything is a file](/concepts/everything-is-a-file).
- **export / import** — A node *exports* its namespace as a 9P server; a peer *imports* it as local files. Import is the historically missing half of 9P that the mesh restores. See [import, export, and /n](/concepts/import-export-and-n) and [the missing half of 9P](/concepts/missing-half-of-9p).
- **grant** — The durable record "this peer may bind this subtree with these rights," matched exactly by peer and attach name (no wildcards); it produces an `Authorization`, not an ACL. See [attach policy](/concepts/attach-policy). (`crates/wanix-id/src/grant.rs:31-34`.)
- **mesh** — The cross-machine layer: 9P over iroh QUIC, the only async edge of the workspace. See [9P over iroh QUIC](/concepts/9p-over-iroh-quic) and [the crate map](/reference/crate-map-and-layering).
- **namespace** — A per-process tree assembled from binds; resolution walks it. See [per-process namespaces](/concepts/per-process-namespaces) and [namespace resolution](/concepts/namespace-resolution).
- **`/n/<peer>`** — The conventional mount point for an imported peer namespace. Note: the shipped CLI binds a single slot `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` is designed-but-unshipped, so treat `/n/<peer>` as a labelled convention. See [import, export, and /n](/concepts/import-export-and-n).
- **plumber** — The `#plumb` topic bus; `<topic>/send` publishes newline-JSON, `<topic>/recv` drains. See [#plumb](/devices/plumb).
- **venti** — Plan 9's content-addressed archival store; Wanix's `#cas` and `.wcap` capsules are venti made Wanix-shaped (BLAKE3-keyed blobs). See [#cas](/devices/cas) and [content-addressed data plane](/concepts/content-addressed-data-plane). (`crates/wanix-cas/src/lib.rs:1`.)

## Typed primitives

- **9P** — The single wire contract for every Wanix filesystem, local or remote (Tversion/Tattach/Twalk/Tread/Twrite and friends), split between a dependency-free protocol crate and a server crate. See [the 9P contract](/concepts/the-9p-contract) and [protocol vs. server split](/concepts/protocol-vs-server-split). (`crates/wanix-protocol/src/lib.rs`.)
- **ALPN** — The QUIC application-layer protocol negotiation byte string that selects which Wanix plane a dialed connection speaks. Each plane has its own. See [9P over iroh QUIC](/concepts/9p-over-iroh-quic).
- **`wanix/9p/1`** — The ALPN constant for the 9P control plane over QUIC: `WANIX_9P_ALPN`. (`crates/wanix-mesh/src/lib.rs:68`.) See [9P over iroh QUIC](/concepts/9p-over-iroh-quic).
- **`wanix/cpu/1`** — The ALPN constant for the cpu exec plane, distinct from the 9P plane; a cpu connection multiplexes its own control + reverse-9P channels. (`crates/wanix-mesh/src/cpu.rs:39`.) See [#cpu](/devices/cpu) and [send the agent to the data](/concepts/send-agent-to-the-data).
- **`Authorization`** — The outcome of authorizing one attach: a scoped root (a `SubtreeFs`) plus the `Rights` enforced on it; this is what an `AttachPolicy` returns and the 9P server installs as the fid root. (`crates/wanix-id/src/grant.rs:16-21`.) See [attach policy](/concepts/attach-policy) and [the attach & capability contract](/reference/attach-and-capability-contract).
- **`AttachPolicy`** — The trait `evaluate(peer, aname) -> Option<Authorization>`; `None` denies (default-deny) and binds nothing, keeping "who gets which root" in one auditable place out of the wire code. (`crates/wanix-id/src/policy.rs:10-15`.) See [attach policy](/concepts/attach-policy).
- **`BindPosition`** — The enum choosing where a new binding lands at a destination: `First` (default, in front), `Replace`, or `Last`. (`crates/wanix-vfs/src/binding.rs:9-17`.) See [namespace binding](/concepts/namespace-binding).
- **`confine_to_prefix`** — The `FileSystem` hook a `SubtreeFs` calls on every symlink-dereferencing path so an in-prefix symlink cannot escape the granted subtree; it canonicalizes on host-backed filesystems and denies any target outside the prefix. (`crates/wanix-vfs/src/subtree.rs:71-74`.) See [SubtreeFs confines to a prefix](/concepts/subtreefs-confine-to-prefix).
- **`ContentHash`** — A 32-byte BLAKE3 digest newtype living at the bottom of the graph (`wanix-fs`) so the filesystem hook, `#cas`, and the mesh data plane name the same type without depending on each other; equal bytes share a hash, which is the dedup and end-to-end-verification property. (`crates/wanix-fs/src/content_hash.rs:21-35`.) See [end-to-end hash verification](/concepts/end-to-end-hash-verification) and [#cas](/devices/cas).
- **`EACCES`** — The 9P permission error a denied attach replies with; default-deny means an unmatched grant yields `EACCES` and no bound fid. (`crates/wanix-9p/src/lib.rs:131`.) See [attach policy](/concepts/attach-policy).
- **`EndpointId`** — iroh's verified 32-byte node public key on the QUIC layer; Wanix derives its own `PeerId` from it (`peer_id_for(endpoint_id)`), so the network identity and the Wanix identity are the same key on two planes. (`crates/wanix-mesh/src/identity.rs:23-29`.) See [the key is the address](/concepts/key-is-the-address) and [one identity, two planes](/concepts/one-identity-two-planes).
- **fid** — A 9P file identifier: a per-connection handle the client binds to a path at attach/walk and then reads, writes, and clunks; the server keeps fid state. (`crates/wanix-9p/src/lib.rs:88-95`.) See [the 9P contract](/concepts/the-9p-contract).
- **`FsError::NotSupported`** — The `FileSystem` error meaning "this operation is not supported by this file or filesystem" (one variant of the shared `FsError` enum that also carries `NotFound`, `PermissionDenied`, `IsDirectory`, and friends). (`crates/wanix-fs/src/error.rs:9-42`.) See [the FileSystem trait](/concepts/the-filesystem-trait) and [the FileSystem reference](/reference/filesystem-trait).
- **`msize`** — The negotiated maximum 9P message size in bytes; the server caps the client's request at `DEFAULT_MAX_MSIZE` (131072) and read/write counts are clamped to fit under it. (`crates/wanix-9p/src/lib.rs:53`, `crates/wanix-9p/src/session.rs:25`.) See [the 9P contract](/concepts/the-9p-contract) and [serve and discovery](/reference/serve-and-discovery).
- **`NormalizedPath`** — The validated, canonical path newtype every Wanix filesystem operation takes; a string that has already been checked and normalized, so paths cross the trust boundary typed rather than raw. (`crates/wanix-fs/src/path.rs:7-63`.) See [NormalizedPath](/concepts/normalizedpath).
- **`PeerId`** — A verified peer's raw 32-byte ed25519 public key newtype; the unit a grant is keyed by and the address by which you name a node. (`crates/wanix-id/src/peer.rs:3-20`.) See [the key is the address](/concepts/key-is-the-address) and [persisted ed25519 identity](/concepts/persisted-ed25519-identity).
- **`Rights`** — Two additive capability bits, `read` and `write`, that a `SubtreeFs` consults centrally (reads need `read`; every mutation and write-open needs `write`); constructed via `read_only()`, `read_write()`, or `none()`. Granting write does not imply read. (`crates/wanix-vfs/src/subtree.rs:8-49`.) See [attach policy](/concepts/attach-policy).
- **`SubtreeFs`** — A `FileSystem` re-rooted at a subpath of a backing filesystem and gated by `Rights`; this is the concrete form of a capability grant, so a grant is a re-rooted subtree, not an ACL, and the rights are enforced inside the filesystem itself, not only at attach time. (`crates/wanix-vfs/src/subtree.rs:58-66`.) See [SubtreeFs confines to a prefix](/concepts/subtreefs-confine-to-prefix) and [capability is a bind](/concepts/capability-is-a-bind).
- **`Tauth`** — The 9P authentication request; Wanix answers it `ENOSYS` (errno 38) and binds no fid — there is no 9P auth handshake, because identity lives in the QUIC/ed25519 layer below the protocol. (`crates/wanix-9p/src/dispatch.rs:45`, `crates/wanix-9p/src/error.rs:15`, `crates/wanix-9p/src/lib.rs:375-379`.) See [Tauth is ENOSYS](/concepts/tauth-is-enosys).

## See also

- [The 9P contract](/concepts/the-9p-contract) — the wire vocabulary most of these primitives serve.
- [Attach policy](/concepts/attach-policy) — where `AttachPolicy`, `Authorization`, `Rights`, and `EACCES` meet.
- [NormalizedPath](/concepts/normalizedpath) — the typed-path boundary in detail.
- [Capability is a bind](/concepts/capability-is-a-bind) — why a grant is a `SubtreeFs`, not an ACL.
- [Concepts index](/find/concepts) and [tags index](/find/tags) — the other two ways to find a page.
- [Crate map & layering](/reference/crate-map-and-layering) — which crate owns each type.

## Status / honest limits

- A glossary one-liner is a pointer, not the contract. The linked page and the cited `crate/src/file.rs:line` reference are authoritative; if they disagree with the sentence here, they win.
- `Tauth` is answered `ENOSYS` and binds no fid (`crates/wanix-9p/src/lib.rs:375-379`). There is no in-protocol 9P authentication handshake; peer identity is established one layer down, on the QUIC/ed25519 transport.
- A grant is matched exactly by `PeerId` and attach name with no wildcards, and the default is deny (`crates/wanix-id/src/policy.rs:42-44`). `PeerId`-keyed per-peer namespaces are the designed shape, but the shipped CLI mount still binds a single `/n/remote` slot (`crates/wanix-cli/src/mount.rs:26`); read `/n/<peer>` here as a convention, not a guaranteed live path.
- Exec-bearing devices named in these entries — `#task`, `#agent`, `#cpu` — are local-trust only and are not exposed to untrusted or public peers. The isolation is cheap and scalable, not a sandbox for arbitrary untrusted code, and there are no hard CPU/memory limits yet.
