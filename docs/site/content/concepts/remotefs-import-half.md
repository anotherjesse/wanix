---
title: RemoteFs — the Import Half of 9P
slug: concepts/remotefs-import-half
pageType: concept
oneLiner: A synchronous wanix_fs::FileSystem that speaks 9P to a remote server, so binding it splices a remote namespace — files and #-devices alike — into your own.
audience: [developer, visionary]
tags: [mesh, shipped, 9p, import, caveat]
sourceRefs:
  - crates/wanix-9p-client/src/remote.rs:1-265
  - crates/wanix-9p-client/src/fid.rs:115-166
  - crates/wanix-9p-client/src/readdir.rs:1-67
  - crates/wanix-9p-client/src/file.rs:1-77
  - crates/wanix-9p-client/src/conn.rs:187-219
  - crates/wanix-cli/src/mount.rs:26-29
  - docs/mesh-the-missing-half-of-9p.md:451-496
seeAlso:
  - concepts/missing-half-of-9p
  - concepts/import-export-and-n
  - concepts/five-hostile-peer-corrections
  - concepts/streaming-import-fs
  - concepts/9p-over-iroh-quic
prerequisites:
  - concepts/the-9p-contract
  - concepts/protocol-vs-server-split
usedInFlows:
  - {flow: learn/wire-a-mesh, step: 3}
honestLimits:
  - "read_dir walks from the attach root every call; over a WAN a deep tree is O(n^2) round trips, not one batched listing."
  - "A listing is a best-effort snapshot, not atomic: concurrent server-side mutation can shift entries between Treaddir pages."
  - "One request is outstanding at a time per connection; concurrent FileSystem callers serialize on the mutex, they do not pipeline on the wire."
  - "The shipped CLI mount binds a single slot /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is a labelled convention, not yet shipped."
---

# RemoteFs — the Import Half of 9P

A synchronous `wanix_fs::FileSystem` that speaks 9P to a remote server, so binding it splices a remote namespace — files and `#`-devices alike — into your own.

Wanix could always *export*: it served its filesystems as 9P over a process pipe, TCP, a WebSocket. What it could not do was *import* — open a peer's 9P export and read it as if it were a local directory. `RemoteFs` (`crates/wanix-9p-client/src/remote.rs`) is that missing half. It is the photographic negative of the server's `serve_stream`: where the server decodes a `Twalk` and resolves it against a local `FileSystem`, `RemoteFs` *encodes* a `Twalk` and resolves a local path *into* one. And because it is itself a `wanix_fs::FileSystem`, the moment you bind it into a namespace, every resolution into that subtree becomes a 9P exchange on the wire — and the remote tree, regular files and `#`-devices both, is part of your file view.

## Show it: bind a remote at /n/remote and read through it

The shipped `wanix-rust mount` subcommands are the smallest possible proof. One process exports a tree; another dials it, builds a `RemoteFs`, binds it into a fresh `Namespace`, and runs exactly one filesystem operation *through the namespace* (`crates/wanix-cli/src/mount.rs:1-29`):

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

# Process A: export a host directory as 9P over TCP.
wanix-rust p9-listen --addr 127.0.0.1:5640 ./shared &

# Process B: dial it, bind the export at /n/remote, list through the namespace.
wanix-rust mount ls 127.0.0.1:5640 /n/remote
# -> the entries of ./shared, fetched over the wire
```

Nothing in `mount ls` reaches into `./shared` directly. The bytes traverse `Namespace -> RemoteFs -> 9P -> server` and back (`docs/mesh-the-missing-half-of-9p.md:440-449`). What you just did is Plan 9's **import**: `RemoteFs` is the client end of `bind`, `/n/remote` is the mount point, and the remote namespace is now spliced into yours. Swap the TCP transport for a QUIC stream and the same import reaches a peer across the internet — see [9P over iroh QUIC](/concepts/9p-over-iroh-quic).

## It is the negative of the server, sharing one wire format

`RemoteFs` adds no new protocol. It reuses the exact `wanix-protocol` codecs the server already uses — `p9_tlopen`/`p9_decode_rlopen`, `p9_tgetattr`/`p9_decode_rgetattr`, `p9_tmkdir`, `p9_tunlinkat`, `p9_trenameat`, `p9_treaddir` — just calling the encoders the server calls the decoders for, and vice versa (`crates/wanix-9p-client/src/remote.rs:13-16`, `remote/ops.rs:9-12`). Its dependency footprint is deliberately tiny: `wanix-fs` for the trait it implements and `wanix-protocol` for the frames it speaks. It knows nothing about engines, tasks, or the network — its transport is any `Box<dyn Duplex>`, a blocking bidirectional byte stream (`remote.rs:56`). That is why iroh lands "as one more byte stream above a core that never learned its name" (`docs/mesh-the-missing-half-of-9p.md:487-492`). This separation — the wire and codec in `wanix-protocol`, the server and now the client built on top — is the [protocol/server split](/concepts/protocol-vs-server-split).

Each `FileSystem` method follows one shape: walk from the attach root to a guarded scratch fid, issue the matching 9P exchange, let the guard clunk the fid on every exit path (`remote.rs:1-8`). `open` walks then `Tlopen`s (or `Tlcreate`s when `options.create` is set); `metadata` walks then `Tgetattr`s; `create_dir`/`remove_file`/`rename`/`read_link` each walk to a parent fid and issue the addressed mutation.

## Concurrency: one request outstanding, serialized on a mutex

The connection lives behind an `Arc<Mutex<P9Conn>>` (`remote.rs:42`). The server's `serve_stream` is strictly serial — it reads one frame, replies, then reads the next — so the client matches it by keeping a single request outstanding. Concurrent `FileSystem` callers do not race on the wire; they queue on the mutex (`remote.rs:1-8`). This is correct, not clever: a tag pool exists so a long-lived mount never wraps `u16`, but in practice only a tiny number of tags is ever live (`crates/wanix-9p-client/src/fid.rs:71-80`). The cost is honest — concurrent callers do not pipeline.

## The five hostile-peer corrections

A 9P server is, on the mesh, an *untrusted* peer. `RemoteFs` was hardened against a server that lies, stalls, or floods. Five corrections, each grounded in code:

1. **Frame-size ceiling.** Version negotiation offers a preferred `msize`, then clamps the server's reply to a sane window: `version.msize.clamp(P9_HEADER_FLOOR, PREFERRED_MSIZE)` (`crates/wanix-9p-client/src/conn.rs:191`). A server cannot force a 4 GiB frame, and it cannot negotiate below the 512-byte floor that keeps a frame usable (`conn.rs:217-219`).
2. **Honest seekability.** At open time a `Tgetattr` decides whether the fid is a regular, offset-addressable file. Devices and service streams report `is_seekable() == false` and refuse `seek` rather than inventing a fictional offset against a server that ignores `Tread.offset` (`crates/wanix-9p-client/src/file.rs:6-12`, `remote.rs:124-156`).
3. **RAII fid guards.** Every walked fid is a `ScratchFid` that clunks itself on `Drop`, including a panic unwinding through the caller; only when ownership transfers to an open `RemoteFile` is the guard defused via `into_fid` (`crates/wanix-9p-client/src/fid.rs:115-166`, `remote.rs:151-155`). A long-lived mount cannot leak server-side fid state through error paths.
4. **Bounded read_dir.** The `Treaddir` cookie loop caps both total entries (`MAX_ENTRIES = 1_000_000`) and round trips (`MAX_ITERATIONS = 100_000`), and breaks if the server fails to advance its cookie — so a server that streams forever or stalls its cursor cannot hang the importer (`crates/wanix-9p-client/src/readdir.rs:24-65`).
5. **Server-side O_APPEND.** Append is delegated to the server via the `O_APPEND` open flag, so the server seeks to end before each write; the client never races a `Tgetattr`-per-write to compute the offset (`file.rs:6-12`, `remote.rs:138-142`).

These five are the substance of the page on [hostile-peer corrections](/concepts/five-hostile-peer-corrections): the import half assumes the export is adversarial.

## One round trip instead of two: Twalkgetattr

Negotiation offers `9P2000.L.Google.2`, which unlocks the `Twalkgetattr` extension (`conn.rs:187-192`). When the server speaks it, `metadata` collapses walk-then-stat into a single `Twalkgetattr`, saving a round trip; otherwise a `Twalk` is followed by a separate `Tgetattr` on the resulting fid (`remote.rs:106-122`). `supports_walkgetattr()` exposes the negotiated capability so a caller can branch on it (`remote.rs:92-99`). On a high-latency link, halving the round trips of the most common operation — "stat this path" — is the difference between a browsable mount and a sluggish one.

## See also

- [The missing half of 9P](/concepts/missing-half-of-9p) — why import, not export, was the keystone the mesh was waiting on.
- [Import, export, and /n](/concepts/import-export-and-n) — the Plan 9 vocabulary `RemoteFs` realizes, and the `/n/<peer>` mount convention.
- [The five hostile-peer corrections](/concepts/five-hostile-peer-corrections) — the full treatment of treating an export as adversarial.
- [StreamingImportFs](/concepts/streaming-import-fs) — the long-lived, demand-paged mount built on top of this collected-operation core.
- [9P over iroh QUIC](/concepts/9p-over-iroh-quic) — the transport that carries `RemoteFs` to a peer on the open internet.
- [Devices import for free](/concepts/devices-import-for-free) — the payoff: because every `#`-device is a plain `FileSystem`, importing reaches `#kv`, `#agent`, and the rest at once.

## Status / honest limits

- **`read_dir` walks from the attach root every call.** Each operation re-walks from the root to the target before listing (`remote/ops.rs:30-37`). Over a WAN, browsing a deep tree is many round trips, not one batched listing — closer to O(n^2) in path depth across a session than O(1).
- **Listings are best-effort snapshots, not atomic.** The server re-lists from its own directory on each `Treaddir`, so concurrent server-side mutation can shift entries between pages (`readdir.rs:8-13`). Without Google.2, per-entry metadata is listing-only (file type from the directory-entry byte; size and mode are placeholders) — sufficient to browse, not a substitute for a per-file `metadata` call.
- **One request outstanding per connection.** Concurrent `FileSystem` callers serialize on the `Arc<Mutex<P9Conn>>`; they do not pipeline on the wire (`remote.rs:1-8`). High-fan-out parallel reads against one mount run sequentially.
- **`/n/remote` is one shipped slot.** The CLI `mount` subcommands bind a single mount point, `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is the designed convention for naming many imported nodes, but it is a labelled convention here, not yet shipped — treat `/n/<peer>` as a way of *talking* about the mesh, not a path the shipped CLI populates per peer.
