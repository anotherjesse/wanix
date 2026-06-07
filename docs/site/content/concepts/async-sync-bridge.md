---
title: The async/sync Bridge Confined to One Seam
slug: concepts/async-sync-bridge
pageType: concept
oneLiner: iroh and tokio live only in wanix-mesh; BlockingDuplex drives async QUIC streams on a held runtime Handle (never block_on on a worker), so the whole 9P core stays transport-free.
audience: [developer]
tags: [mesh, shipped, caveat]
sourceRefs:
  - crates/wanix-mesh/src/duplex.rs:1-175
  - crates/wanix-mesh/src/lib.rs:1-68
  - crates/wanix-mesh/src/dialer.rs:1-130
  - crates/wanix-mesh/src/handler.rs:101-163
  - crates/wanix-mesh/src/node.rs:1-39
  - AGENTS.md:135-138
seeAlso:
  - concepts/9p-over-iroh-quic
  - concepts/remotefs-import-half
  - reference/crate-map-and-layering
prerequisites:
  - concepts/9p-over-iroh-quic
  - reference/crate-map-and-layering
usedInFlows: []
honestLimits:
  - "Handle::block_on panics if called from a thread that is itself a runtime worker; the bridge is only ever used from non-runtime threads (spawn_blocking pool inbound, ordinary OS threads outbound)."
  - "iroh and tokio are confined to wanix-mesh by convention enforced in review, not by a compiler-checked boundary."
  - "Each live inbound session pins one blocking-pool thread inside block_on for the session's lifetime; concurrency is capped (MAX_CONCURRENT_SESSIONS = 512) rather than made cheap."
canonicalCaveatFor: []
---

# The async/sync Bridge Confined to One Seam

iroh and tokio live only in `wanix-mesh`; `BlockingDuplex` drives async QUIC streams on a held runtime `Handle` (never `block_on` on a worker), so the whole 9P core stays transport-free.

The mesh carries 9P over an async QUIC transport (iroh), but the 9P server and client are strictly synchronous and must never learn what carries their bytes. That tension could leak `async`/`tokio`/`iroh` into a dozen crates. Wanix refuses to let it: the entire async-to-sync conversion happens in one file, `crates/wanix-mesh/src/duplex.rs`, so every other crate — `wanix-fs`, `wanix-vfs`, `wanix-9p`, `wanix-9p-client`, the service devices — stays sync and runtime-free.

## Show: a remote filesystem with no async in sight

When you dial a peer, `MeshDialer::dial` hands back an `Arc<RemoteFs>` — and `RemoteFs` is a plain `wanix_fs::FileSystem` (`crates/wanix-mesh/src/dialer.rs:61-82`). Bind it into a namespace and `read`, `write`, and `read_dir` look exactly like any local device. Yet every one of those calls is secretly a 9P exchange riding a QUIC stream. The dialer's own doc comment states the contract: "the returned `RemoteFs` is a fully synchronous `wanix_fs::FileSystem` whose method calls drive the QUIC stream through the held `Handle`, on non-runtime threads only" (`crates/wanix-mesh/src/dialer.rs:11-13`). Nothing above the mesh ever sees a `Future`.

## Name it: the single-edge rule

The layering rule is explicit (`AGENTS.md:135-138`): `wanix-mesh` is the single async/iroh edge — keep tokio and iroh out of every other crate, including the service devices and the synchronous 9P core that the mesh reuses. The crate's own module doc echoes it: "the synchronous 9P core is reused **unchanged**; the async/sync boundary is bridged here and nowhere else" (`crates/wanix-mesh/src/lib.rs:6-9`). This is a convention enforced in review, not by the compiler — but holding it is what keeps `wanix-fs` portable to native, browser, and capsule contexts that have no runtime at all.

## BlockingReader / BlockingWriter / BlockingDuplex on a held Handle

`duplex.rs` provides three shapes because the two sides of 9P want different things (`crates/wanix-mesh/src/duplex.rs:9-16`):

- `BlockingReader` and `BlockingWriter` each own one stream half. The server's `P9Server::serve_stream` takes a separate `Read` and `Write`, so the inbound path splits the bidi stream into these two.
- `BlockingDuplex` owns both halves and is `Read + Write`, satisfying `wanix_9p_client::Duplex` for the outbound `RemoteFs`.

The trick is the same in all three: each wrapper stores a tokio `Handle` explicitly and turns a blocking `read`/`write` into `handle.block_on(async { ... })` against iroh's async stream (`crates/wanix-mesh/src/duplex.rs:72-122`). A clean stream EOF (`Ok(None)` from iroh) maps to a 0-byte read, the contract a synchronous reader expects (`crates/wanix-mesh/src/duplex.rs:78-79`). An optional per-op deadline is applied *inside* the driven future via `tokio::time::timeout`, because the runtime's time driver must already be entered when the timeout is constructed — building it outside the runtime context panics with "no reactor running" (`crates/wanix-mesh/src/duplex.rs:34-51`).

## Why never `Handle::current().block_on` on a worker thread

`Handle::block_on` **panics** if called from a thread that is itself a runtime worker (`crates/wanix-mesh/src/duplex.rs:18-26`). This is the concrete bridge hazard the blueprint flagged that earlier designs hand-waved. Two disciplines defuse it:

1. The wrappers hold a `Handle` explicitly rather than calling `Handle::current()`. The runtime is owned by the `MeshNode` and "never entered on the caller's thread" (`crates/wanix-mesh/src/node.rs:9-12`), so the bridge works with no runtime entered on the calling thread.
2. The bridge types are only ever used from non-runtime threads.

That second discipline drives where each direction runs.

## Inbound in spawn_blocking; outbound on non-runtime threads

**Inbound** (serving a peer): `P9ProtocolHandler::accept` reads the verified peer id, acquires a session permit, then runs the unchanged synchronous `serve_stream` inside `tokio::task::spawn_blocking` — the blocking pool, not a runtime worker (`crates/wanix-mesh/src/handler.rs:128-132`). The comment is exact: "Run each stream on a blocking-pool thread so a long-lived session never pins a runtime worker and the `BlockingReader/Writer` can `block_on` safely" (`crates/wanix-mesh/src/handler.rs:123-127`). Each stream is detached, so a blocked read on one never stalls another. The server's read is the idle wait for the next request — 9P has no keepalive — so the per-op deadline rides only on the write half; applying it to the idle read would tear down a healthy mounted-but-idle session (`crates/wanix-mesh/src/handler.rs:149-159`).

**Outbound** (importing a peer): `RemoteFs`'s `FileSystem` calls run on ordinary OS or `spawn_blocking` threads — never a runtime worker. The dialer's `open_stream` does its connect/`open_bi` work with a `block_on` on the held handle (legitimate, because the dialer call itself runs off-runtime), then wraps the resulting halves in a `BlockingDuplex` (`crates/wanix-mesh/src/dialer.rs:114-129`). One subtlety the dialer encodes: an iroh bidi stream is invisible to the peer's `accept_bi` until the opener writes its first byte, so `RemoteFs` writing the 9P `Tversion` immediately is exactly what makes the inbound side see the stream — a dialer that read first would hang (`crates/wanix-mesh/src/dialer.rs:5-9`, `crates/wanix-mesh/src/duplex.rs:126-130`).

## Under the hood: duplex.rs is the single seam

The whole conversion is ~175 lines (`crates/wanix-mesh/src/duplex.rs`). Everything else — frame splitting, fids, walk/stat/read/write semantics, EOF rules — is the same synchronous code the local `p9-listen` and `p9-stdio` transports run. The mesh did not fork the protocol to go remote; it added one async-aware byte pipe and reused the rest. That is why `RemoteFs` is "also a `FileSystem`" the moment the duplex is built, and why a peer's `#kv`, `#cas`, or `#agent` imports across the mesh for free.

## See also

- [9P over iroh QUIC](/concepts/9p-over-iroh-quic) — the transport this seam bridges, with ALPN and verified identity.
- [RemoteFs is the import half](/concepts/remotefs-import-half) — the synchronous `FileSystem` the outbound bridge backs.
- [Crate map and layering](/reference/crate-map-and-layering) — where `wanix-mesh` sits and why it is the only async crate.
- [Devices import for free](/concepts/devices-import-for-free) — the payoff of keeping every device a plain sync `FileSystem`.

## Status / honest limits

- `Handle::block_on` panics if called from a thread that is itself a runtime worker. The bridge is correct only because it is used exclusively from non-runtime threads: inbound on the `spawn_blocking` pool, outbound on ordinary OS threads (`crates/wanix-mesh/src/duplex.rs:18-26`).
- The async-only boundary is held by convention enforced in review (`AGENTS.md:135-138`), not by a compiler-checked import barrier. Nothing stops a crate from adding a tokio dependency except discipline.
- Each live inbound session pins one blocking-pool thread inside `block_on` on the idle read for the session's lifetime; this is bounded by a hard cap (`MAX_CONCURRENT_SESSIONS = 512`) paired with a per-op write deadline, not made free (`crates/wanix-mesh/src/node.rs:30-39`).
- The per-op deadline does not bound the server's idle read between requests (9P has no keepalive), so a mounted-but-idle session legitimately parks a thread until the peer sends its next request or drops the connection (`crates/wanix-mesh/src/handler.rs:149-159`).
