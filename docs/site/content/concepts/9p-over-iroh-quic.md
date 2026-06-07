---
title: 9P over iroh QUIC under One ALPN
slug: concepts/9p-over-iroh-quic
pageType: concept
oneLiner: One iroh Endpoint per node carries 9P (ALPN wanix/9p/1) over QUIC bidi streams; the synchronous 9P core runs unchanged behind a held-runtime blocking bridge.
audience: [developer]
tags: [mesh, transport, iroh, quic, shipped, caveat]
sourceRefs:
  - crates/wanix-mesh/src/lib.rs:63-68
  - crates/wanix-mesh/src/node.rs:27-124
  - crates/wanix-mesh/src/node.rs:354-368
  - crates/wanix-mesh/src/handler.rs:101-163
  - crates/wanix-mesh/src/duplex.rs:17-51
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

# 9P over iroh QUIC under One ALPN

One iroh Endpoint per node carries 9P (ALPN `wanix/9p/1`) over QUIC bidi streams; the synchronous 9P core runs unchanged behind a held-runtime blocking bridge.

The mesh page told you that binding a peer's namespace at `/n/<peer>` turns every file resolution into a 9P exchange "on the wire." This page is the wire. It answers a single question: how does the same synchronous `P9Server` that serves a Unix socket end up speaking 9P across the internet, through NATs, without the 9P code learning a word of async? The answer is one QUIC endpoint per node, one ALPN string that selects the 9P plane, and a narrow blocking bridge that is the *only* place tokio and iroh touch the runtime.

## Show: one node, one endpoint, one ALPN

A node is a `MeshNode`. It owns a dedicated multi-thread tokio runtime, an `iroh::Endpoint` bound from the node's persisted ed25519 secret key, and — once serving — a `Router` that dispatches one ALPN to a 9P handler (`crates/wanix-mesh/src/node.rs:64-71`). The ALPN is a literal nine bytes:

```rust
pub const WANIX_9P_ALPN: &[u8] = b"wanix/9p/1";
```

Both ends speak exactly this string. It is the wire contract that selects the 9P control plane on an endpoint that may also be carrying other planes (`crates/wanix-mesh/src/lib.rs:63-68`). When a dialer connects, QUIC's ALPN negotiation routes the connection to the `P9ProtocolHandler`; when the blob data plane shares the same endpoint, its own ALPN (`iroh-blobs`) routes blob fetches to a different handler on the same identity. That co-tenancy is its own idea — see [one identity, two planes](/concepts/one-identity-two-planes).

The QUIC peer *is* the cryptographic identity. iroh binds the endpoint from the node's secret key, so the peer id read off the handshake is a proven ed25519 public key, not a claimed name. The dialing address — the "ticket" — is an `EndpointAddr`: the peer's id plus the direct addresses and relay it currently knows. That the key is the address is the whole authorization story; see [the key is the address](/concepts/key-is-the-address).

## Public vs local: two ways onto the network

A node binds one of two ways (`crates/wanix-mesh/src/node.rs:80-124`, `354-368`):

- `MeshNode::bind` joins the **public** iroh network using the n0 preset — relays and DNS-based address lookup — so peers can reach it across NATs and changing IPs. This is the "ticket someone a node id and they can dial you from anywhere" form.
- `MeshNode::bind_local` is the **offline, direct-address-only** form: relays and DNS disabled, the socket pinned to one address. This is the testable form and the form for a LAN where peers exchange direct `EndpointAddr` tickets by hand.

Both bindings register `WANIX_9P_ALPN` up front, so an endpoint advertises the 9P plane the moment it is bound. The local form is what the integration tests and two-process demos use, because it needs no relay infrastructure and its connectivity is deterministic.

## Name: the bridge keeps the 9P core synchronous

Here is the engineering discipline that makes this work. The 9P core — `P9Server::serve_stream` on the inbound side, `RemoteFs` on the outbound side — is strictly synchronous and must never learn about async (`crates/wanix-mesh/src/duplex.rs:1-26`). iroh's stream halves are async. The seam between them lives in exactly one module and nowhere else.

**Inbound.** On each accepted connection the `P9ProtocolHandler` reads the verified peer id from the QUIC handshake — before any `Tattach` is served, because 0-RTT is never used (`crates/wanix-mesh/src/handler.rs:101-106`). Then for every accepted bidi stream it runs the unchanged synchronous `serve_stream` inside `tokio::task::spawn_blocking`, wrapping the split stream halves in a `BlockingReader` and `BlockingWriter` (`crates/wanix-mesh/src/handler.rs:128-162`). Each stream is detached and serves independently, so a blocked read on one stream never stalls another on the same connection.

**Outbound.** `MeshDialer::dial` connects, opens one bidi stream, wraps it in a `BlockingDuplex`, and hands it to `RemoteFs`. `RemoteFs` then writes `Tversion` — and because an iroh bidi stream is invisible to the peer's `accept_bi` until the opener writes its first byte, that `Tversion` write is exactly what makes the inbound side see the stream. A dialer that read first would hang.

The bridge's one hazard is that `Handle::block_on` panics if called from a runtime worker thread. The bridge sidesteps it structurally: the inbound server runs on the blocking pool (not a worker), the outbound `FileSystem` calls run on ordinary OS threads, and each bridge holds an explicit `Handle` rather than calling `Handle::current` (`crates/wanix-mesh/src/duplex.rs:17-51`). The full mechanics of that held-runtime blocking bridge are their own page — see [the async/sync bridge](/concepts/async-sync-bridge).

## The slow-peer budget: a cap and a deadline

Every live inbound session pins one blocking-pool thread inside `block_on` on the server's idle read for the session's lifetime. Left unbounded, a flood of half-open mounts would exhaust the process. Two numbers close that:

- `MAX_CONCURRENT_SESSIONS = 512` — a hard cap, enforced by a semaphore that hands one owned permit per live stream and releases it when the session ends (`crates/wanix-mesh/src/node.rs:30-39`, `crates/wanix-mesh/src/handler.rs:114-132`). The runtime's blocking pool is sized to admit this many sessions plus headroom.
- `DEFAULT_OP_DEADLINE = 30s` — a per-operation deadline that bounds in-flight work (`crates/wanix-mesh/src/node.rs:27-28`).

The deadline is applied asymmetrically on purpose. 9P is strict request/response with no keepalive, so a healthy, mounted-but-idle session blocks indefinitely on the server's read between operations. Putting the deadline on that read would tear down a perfectly alive mount after a brief pause — the "open a mount, walk away for a minute" case. So the inbound deadline rides only on the server's *write*, where a stalled peer must not park a thread forever; the idle read is not deadline-bound (`crates/wanix-mesh/src/handler.rs:144-159`). The client side keeps the deadline on both halves, because its read always follows a request write, which makes 30s-per-RPC a sane response timeout.

## The iroh version reality

The dependency wave is pinned and the reason is documented in the manifest (`crates/wanix-mesh/Cargo.toml:13-24`): iroh-blobs 0.102 depends on iroh `1.0.0-rc.1`, and the blob data plane must share one endpoint and `Router` with the 9P control plane, so both planes are locked to the same iroh wave. The resolved set is `iroh = "=1.0.0-rc.1"`, `iroh-blobs = "0.102.0"`, `iroh-gossip = "0.100.0"`. The pin is `=` on iroh deliberately: the planes cannot drift across iroh versions and still share an endpoint.

## See also

- [The async/sync bridge](/concepts/async-sync-bridge) — the held-runtime `block_on` mechanics and the runtime-worker hazard in full.
- [The key is the address](/concepts/key-is-the-address) — why the QUIC peer id *is* the authorization principal.
- [One identity, two planes](/concepts/one-identity-two-planes) — how the 9P control plane and the blob data plane share one endpoint.
- [The streaming import FS](/concepts/streaming-import-fs) — why a never-EOF service read gets its own bidi stream.
- [Import, export, and /n](/concepts/import-export-and-n) — the namespace half this transport carries.

## Status / honest limits

- **iroh is a release-candidate wave.** The transport is pinned to iroh `=1.0.0-rc.1`; the data plane rides iroh-blobs `0.102.0`, which upstream flags experimental. The pin is intentional (the two planes share an endpoint and cannot drift), but it is an `rc` wave, not a stable release (`crates/wanix-mesh/Cargo.toml:13-24`).
- **Public binding joins n0 infrastructure.** `bind` uses the n0 relay/DNS preset to traverse NATs; the deterministic, offline form is `bind_local` with hand-exchanged direct `EndpointAddr` tickets (`crates/wanix-mesh/src/node.rs:80-124`). A freshly bound public node is not reliably dialable for ~2s, so first contact should prefer a ticket carrying direct addresses.
- **One frame at a time per stream.** The bridge does not change 9P's request/response shape: a served stream handles one frame at a time, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on that same stream. The deadlock-safe answer is a second stream — see [the single-frame serve caveat](/concepts/single-frame-serve-caveat) and [the streaming import FS](/concepts/streaming-import-fs).
- **Tauth stays ENOSYS.** This transport authenticates by the verified QUIC peer id, not a 9P auth handshake; there is no `Tauth` exchange — see [Tauth is ENOSYS](/concepts/tauth-is-enosys). Exposing exec planes (`#cpu`, `#task`, `#agent`) to untrusted peers is not claimable here; see [safe for untrusted is not claimable](/concepts/safe-for-untrusted-not-claimable).
