---
title: The FileSystem Contract over iroh QUIC
slug: concepts/9p-over-iroh-quic
pageType: concept
oneLiner: One iroh Endpoint per node carries the FileSystem contract over QUIC bidi streams on two ALPNs — the native wire (wanix/fs/1) is the default Wanix↔Wanix plane, 9P (wanix/9p/1) is the foreign-edge plane; the synchronous cores run unchanged behind a held-runtime blocking bridge.
audience: [developer]
tags: [mesh, transport, iroh, quic, shipped, caveat]
sourceRefs:
  - crates/wanix-mesh/src/lib.rs:63-97
  - crates/wanix-mesh/src/node.rs:27-124
  - crates/wanix-mesh/src/node.rs:354-368
  - crates/wanix-mesh/src/handler.rs:101-163
  - crates/wanix-mesh/src/wire_handler.rs
  - crates/wanix-mesh/src/dialer.rs:71-150
  - crates/wanix-mesh/src/duplex.rs:17-51
  - crates/wanix-mesh-wire/src/lib.rs
  - crates/wanix-mesh/Cargo.toml:13-24
seeAlso:
  - concepts/async-sync-bridge
  - concepts/key-is-the-address
  - concepts/one-identity-two-planes
  - concepts/streaming-import-fs
  - concepts/import-export-and-n
prerequisites:
  - concepts/import-export-and-n
usedInFlows:
  - {flow: wire-a-mesh, step: 2}
honestLimits:
  - "iroh is pinned to a release-candidate wave (=1.0.0-rc.1); the blob data plane rides iroh-blobs 0.102, which upstream flags experimental."
  - "Public binding joins the n0 relay/DNS network; the testable, offline form is bind_local with direct EndpointAddr tickets only."
  - "A served 9P stream still handles one frame at a time, so a blocking recv cannot interleave with a write on that same stream — see the single-frame caveat."
---

# The FileSystem Contract over iroh QUIC

One iroh Endpoint per node carries the `FileSystem` contract over QUIC bidi streams on two ALPNs — the native wire (`wanix/fs/1`) is the default Wanix↔Wanix plane, 9P (`wanix/9p/1`) is the foreign-edge plane; the synchronous cores run unchanged behind a held-runtime blocking bridge.

The mesh page told you that binding a peer's namespace at `/n/<peer>` turns every file resolution into an exchange "on the wire." This page is the wire. It answers a single question: how does a synchronous Wanix `FileSystem` — the same one that serves a Unix socket — end up reachable across the internet, through NATs, without the filesystem code learning a word of async? The answer is one QUIC endpoint per node, an ALPN string that selects the plane, and a narrow blocking bridge that is the *only* place tokio and iroh touch the runtime.

Two planes ride that one endpoint. Between two Wanix nodes the default is the **native FileSystem-over-iroh wire** (`wanix-mesh-wire`, ALPN `wanix/fs/1`): one `postcard`-framed bidi stream per filesystem call and one dedicated stream per open file, with `FsError` crossing as a typed `WireFsError` and no tags or `msize`. The **9P plane** (ALPN `wanix/9p/1`) is the foreign edge — Linux `v9fs`, v86/QEMU virtio-9p, external 9P tooling. Both are thin codecs over the *same* `FileSystem` contract, and both reuse the *same* blocking bridge; this page covers the shared transport mechanics and uses 9P for the worked example because its session core is the older, more-documented one. The native wire's frame shape and its typed-error/per-stream wins are [their own concept](/concepts/missing-half-of-9p); ADR 0004 records why the native wire is the hand-rolled frame rather than 9P or `irpc`.

## Show: one node, one endpoint, ALPNs select the plane

A node is a `MeshNode`. It owns a dedicated multi-thread tokio runtime, an `iroh::Endpoint` bound from the node's persisted ed25519 secret key, and — once serving — a `Router` that dispatches each ALPN to its handler (`crates/wanix-mesh/src/node.rs:64-71`). The ALPNs are literal byte strings:

```rust
pub const WANIX_FS_ALPN: &[u8] = b"wanix/fs/1";   // the native FileSystem wire (default)
pub const WANIX_9P_ALPN: &[u8] = b"wanix/9p/1";   // the foreign-edge 9P plane
```

Both ends speak exactly one of these strings, and it is the wire contract that selects which plane an endpoint serves (`crates/wanix-mesh/src/lib.rs:63-97`). When a `dial_native` connects on `wanix/fs/1`, QUIC's ALPN negotiation routes it to the `NativeFsHandler` (`crates/wanix-mesh/src/wire_handler.rs`); a `wanix/9p/1` connection routes to the `P9ProtocolHandler`; when the blob data plane shares the same endpoint, its own ALPN (`iroh-blobs`) routes blob fetches to a different handler on the same identity. That co-tenancy is its own idea — see [one identity, two planes](/concepts/one-identity-two-planes).

The QUIC peer *is* the cryptographic identity. iroh binds the endpoint from the node's secret key, so the peer id read off the handshake is a proven ed25519 public key, not a claimed name. The dialing address — the "ticket" — is an `EndpointAddr`: the peer's id plus the direct addresses and relay it currently knows. That the key is the address is the whole authorization story; see [the key is the address](/concepts/key-is-the-address).

## Public vs local: two ways onto the network

A node binds one of two ways (`crates/wanix-mesh/src/node.rs:80-124`, `354-368`):

- `MeshNode::bind` joins the **public** iroh network using the n0 preset — relays and DNS-based address lookup — so peers can reach it across NATs and changing IPs. This is the "ticket someone a node id and they can dial you from anywhere" form.
- `MeshNode::bind_local` is the **offline, direct-address-only** form: relays and DNS disabled, the socket pinned to one address. This is the testable form and the form for a LAN where peers exchange direct `EndpointAddr` tickets by hand.

Both bindings register the serving ALPNs up front, so an endpoint advertises its planes the moment it is bound. The local form is what the integration tests and two-process demos use, because it needs no relay infrastructure and its connectivity is deterministic.

## Name: the bridge keeps the synchronous cores synchronous

Here is the engineering discipline that makes this work. The synchronous cores — `wanix-mesh-wire`'s `serve_one`/`NativeFs` on the native plane, `P9Server::serve_stream`/`RemoteFs` on the 9P plane — are strictly synchronous and must never learn about async (`crates/wanix-mesh/src/duplex.rs:1-26`). iroh's stream halves are async. The seam between them lives in exactly one module and nowhere else, and *both* planes ride the same `BlockingDuplex`.

**Inbound (native plane).** On each accepted connection the `NativeFsHandler` reads the verified peer id from the QUIC handshake, binds it to a principal-scoped `FileSystem` view via `AttachPolicy` (default-deny), and for every accepted bidi stream runs the synchronous `wanix_mesh_wire::serve_one` inside `tokio::task::spawn_blocking` over a `BlockingDuplex` (`crates/wanix-mesh/src/wire_handler.rs`). One stream is one filesystem op (one-shot) or one open file (streaming). Each stream is detached, so a blocked read on a never-EOF open file never stalls another stream on the same connection.

**Inbound (9P plane).** The same shape: the `P9ProtocolHandler` reads the verified peer id — before any `Tattach`, because 0-RTT is never used (`crates/wanix-mesh/src/handler.rs:101-106`) — then runs the unchanged synchronous `serve_stream` inside `spawn_blocking`, wrapping the split halves in a `BlockingReader` and `BlockingWriter` (`crates/wanix-mesh/src/handler.rs:128-162`).

**Outbound.** `MeshDialer::dial_native` connects on `wanix/fs/1`, opens a fresh bidi stream per op, wraps it in a `BlockingDuplex`, and drives the `NativeFs` codec over it (`crates/wanix-mesh/src/dialer.rs:109-150`); `MeshDialer::dial` does the same for 9P over `wanix/9p/1`. On either plane the dialer writes the first frame immediately — and because an iroh bidi stream is invisible to the peer's `accept_bi` until the opener writes its first byte, that first write is exactly what makes the inbound side see the stream. A dialer that read first would hang.

The bridge's one hazard is that `Handle::block_on` panics if called from a runtime worker thread. The bridge sidesteps it structurally: the inbound server runs on the blocking pool (not a worker), the outbound `FileSystem` calls run on ordinary OS threads, and each bridge holds an explicit `Handle` rather than calling `Handle::current` (`crates/wanix-mesh/src/duplex.rs:17-51`). The full mechanics of that held-runtime blocking bridge are their own page — see [the async/sync bridge](/concepts/async-sync-bridge).

## The slow-peer budget: a cap and a deadline

Every live inbound session pins one blocking-pool thread inside `block_on` on the server's idle read for the session's lifetime. Left unbounded, a flood of half-open mounts would exhaust the process. Two numbers close that:

- `MAX_CONCURRENT_SESSIONS = 512` — a hard cap, enforced by a semaphore that hands one owned permit per live stream and releases it when the session ends (`crates/wanix-mesh/src/node.rs:30-39`, `crates/wanix-mesh/src/handler.rs:114-132`). The runtime's blocking pool is sized to admit this many sessions plus headroom.
- `DEFAULT_OP_DEADLINE = 30s` — a per-operation deadline that bounds in-flight work (`crates/wanix-mesh/src/node.rs:27-28`).

The deadline is applied asymmetrically on purpose, and identically on both planes. The wire is strict request/response with no keepalive, so a healthy, mounted-but-idle session — or, on the native plane, a live-but-quiet open file like `#agent/<id>/events` waiting for the next event — blocks indefinitely on the server's read. Putting the deadline on that read would tear down a perfectly alive mount or a healthy subscription after a brief pause. So the inbound deadline rides only on the server's *write*, where a stalled peer must not park a thread forever; the idle read is not deadline-bound (`crates/wanix-mesh/src/handler.rs:144-159`, and the native plane carries the same nuance — the idle `FileOp` read is never deadline-bound, only in-flight writes are). The client side keeps the deadline on both halves, because its read always follows a request write, which makes 30s-per-RPC a sane response timeout.

## The iroh version reality

The dependency wave is pinned and the reason is documented in the manifest (`crates/wanix-mesh/Cargo.toml:13-24`): iroh-blobs 0.102 depends on iroh `1.0.0-rc.1`, and the blob data plane must share one endpoint and `Router` with the control planes, so all planes are locked to the same iroh wave. The resolved set is `iroh = "=1.0.0-rc.1"`, `iroh-blobs = "0.102.0"`, `iroh-gossip = "0.100.0"`. The pin is `=` on iroh deliberately: the planes cannot drift across iroh versions and still share an endpoint. (The native-wire codec crate `wanix-mesh-wire` itself depends on *none* of this — it is async-free and iroh-free; only `wanix-mesh` binds it to QUIC.)

## See also

- [The async/sync bridge](/concepts/async-sync-bridge) — the held-runtime `block_on` mechanics and the runtime-worker hazard in full.
- [The key is the address](/concepts/key-is-the-address) — why the QUIC peer id *is* the authorization principal.
- [One identity, two planes](/concepts/one-identity-two-planes) — how the control planes and the blob data plane share one endpoint.
- [The missing half of 9P](/concepts/missing-half-of-9p) — the native FileSystem-over-iroh wire this plane carries, and why every open file is its own stream by construction.
- [The streaming import FS](/concepts/streaming-import-fs) — the 9P-plane workaround (retired on the native plane) that gave a never-EOF read its own bidi stream.
- [Import, export, and /n](/concepts/import-export-and-n) — the namespace half this transport carries.

## Status / honest limits

- **iroh is a release-candidate wave.** The transport is pinned to iroh `=1.0.0-rc.1`; the data plane rides iroh-blobs `0.102.0`, which upstream flags experimental. The pin is intentional (the two planes share an endpoint and cannot drift), but it is an `rc` wave, not a stable release (`crates/wanix-mesh/Cargo.toml:13-24`).
- **Public binding joins n0 infrastructure.** `bind` uses the n0 relay/DNS preset to traverse NATs; the deterministic, offline form is `bind_local` with hand-exchanged direct `EndpointAddr` tickets (`crates/wanix-mesh/src/node.rs:80-124`). A freshly bound public node is not reliably dialable for ~2s, so first contact should prefer a ticket carrying direct addresses.
- **The single-frame caveat is 9P-plane only.** On the 9P plane a served stream handles one frame at a time, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on that same stream — see [the single-frame serve caveat](/concepts/single-frame-serve-caveat). The **native plane does not have this constraint**: every open file rides its own QUIC stream, so a blocking read never blocks a sibling op (the `StreamingImportFs` workaround is retired there). The WebSocket-served 9P door the cockpit uses keeps the single-frame caveat.
- **Identity is from the transport, not the wire.** Both planes authenticate by the verified QUIC peer id; the native wire never carries a principal in its payload, and 9P's `Tauth` stays ENOSYS — there is no in-band auth handshake (see [Tauth is ENOSYS](/concepts/tauth-is-enosys)). On the native plane the verified `remote_id()` binds a per-connection principal-scoped `FileSystem` view. Exposing exec planes (`#cpu`, `#task`, `#agent`) to untrusted peers is not claimable here; see [safe for untrusted is not claimable](/concepts/safe-for-untrusted-not-claimable).
