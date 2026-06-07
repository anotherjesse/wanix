---
title: StreamingImportFs — One Stream per Blocking Open
slug: concepts/streaming-import-fs
pageType: concept
oneLiner: A near-never-EOF read like #agent/<id>/events would freeze the serial import connection, so each blocking streaming open dials its own dedicated bidi stream.
audience: [developer]
tags: [mesh, shipped, caveat, local-trust-only]
sourceRefs:
  - crates/wanix-mesh/src/streaming.rs:1-139
  - crates/wanix-mesh/src/streaming/predicate.rs:20-52
  - docs/mesh-the-missing-half-of-9p.md:126-146
  - docs/mesh-the-missing-half-of-9p.md:300-308
seeAlso:
  - concepts/9p-over-iroh-quic
  - concepts/blocking-stream-eof-contract
  - concepts/import-export-and-n
  - devices/agent
  - devices/plumb
  - concepts/single-frame-serve-caveat
prerequisites:
  - concepts/9p-over-iroh-quic
  - concepts/blocking-stream-eof-contract
usedInFlows: []
honestLimits:
  - A dedicated stream is opened only for read opens that match the predicate; write opens and ordinary files still ride the shared serial import.
  - The default set is exactly #agent/<id>/events, #agent/<id>/reply, and #plumb/<topic>/recv; other never-EOF files need a custom predicate.
  - This is mesh-side framing only; the served side still handles one 9P frame at a time per connection.
---

# StreamingImportFs — One Stream per Blocking Open

A near-never-EOF read like `#agent/<id>/events` would freeze the serial import connection, so each blocking streaming open dials its own dedicated bidi stream.

Import a remote node and most operations are short: a walk, a stat, a read of a small file, a write. They take turns on one connection and nobody notices. But a handful of imported files are *streams* — you `read` and the read parks until something happens on the far side, maybe forever. Put one of those on the shared connection and it parks the whole import. `StreamingImportFs` is the wrapper that keeps those reads from doing that, by giving each one its own pipe.

## The deadlock it avoids

Start from how the import actually talks. `RemoteFs` — the import half of 9P — multiplexes *every* operation onto **one** 9P stream behind an `Arc<Mutex<P9Conn>>`, because the server's `serve_stream` is strictly serial: read one T-message, dispatch, write one R-message, repeat (`docs/mesh-the-missing-half-of-9p.md:126-146`). Concurrent callers serialize on the mutex rather than racing on the wire. For walk, stat, and ordinary reads that is exactly the right shape — the simplest correct thing, matched to the server.

Now open `#agent/<id>/events` across that import and `read` it. The agent has emitted nothing yet, so the `Tread` blocks. The single stream is now parked, and it stays parked: a concurrent `cat #agent/<id>/status`, a `prompt` write, a read of a second agent, a stat of the host root — all of them are stuck behind one read that may never return. The blueprint names this the head-of-line freeze and prescribes the fix bluntly: **one QUIC bidi stream per blocking open-file, not one per attach** (`crates/wanix-mesh/src/streaming.rs:1-24`).

## Which opens need it

Not every file. Opening a fresh QUIC stream for every read would be wasteful, so the wrapper routes only the reads whose blocking is *intrinsic* (`crates/wanix-mesh/src/streaming/predicate.rs:1-8`). The default predicate matches exactly three leaves, anywhere in the path so it works whether the import is bound at `/n/A` or elsewhere:

- `#agent/<id>/events` — the never-EOF normalized event stream;
- `#agent/<id>/reply` — blocks until the latest turn completes;
- `#plumb/<topic>/recv` — the blocking received-envelope stream.

Everything else stays on the shared connection. `#agent/<id>/status`, `#agent/<id>/prompt`, `#plumb/<topic>/send`, `#agent/new`, and a plain `notes/hello.txt` are all short request/response ops, and the predicate returns `false` for them (`crates/wanix-mesh/src/streaming/predicate.rs:37-52`). If you have another never-EOF service file, you supply your own `StreamPredicate` — the set is a default, not a hardcode.

## Each such open dials its own dedicated bidi stream

`StreamingImportFs` is itself a `FileSystem`, so it binds into a namespace at `/n/<node>` exactly like a bare `RemoteFs`. It wraps the shared everyday-ops import plus the `MeshDialer` and the peer address/attach-name needed to dial again. Walk, stat, readdir, mutation, and ordinary opens all go straight to the shared import unchanged (`crates/wanix-mesh/src/streaming.rs:97-139`).

The one method that branches is `open`. A *read* open whose path matches the predicate takes the dedicated path; a write open never does, because a write is a short request/response that never parks the stream (`crates/wanix-mesh/src/streaming.rs:98-106`):

```rust
if options.read && !options.write && (self.predicate)(path) {
    return self.open_dedicated(path, options);
}
self.shared.open(path, options)
```

`open_dedicated` dials a *fresh* `RemoteFs` over a new bidi stream, opens just that one file on it, and returns a handle that owns the dedicated connection (`crates/wanix-mesh/src/streaming.rs:80-94`). The handle, `DedicatedStreamFile`, holds the connection alongside the file: the stream lives as long as the open file, and dropping the file drops both — clunking the fid and tearing the QUIC stream down (`crates/wanix-mesh/src/streaming.rs:141-178`). So a blocking read stalls only its own stream; the shared import keeps serving every other operation.

This is the per-open form of Plan 9's import. You did not get one mount with one channel; you got a mount where the streaming files quietly carve out their own channels, and the head-of-line freeze never happens.

## Under the hood: streaming.rs

The whole mechanism is one small module. `StreamingImportFs` delegates everything except `open` verbatim, and `open` adds exactly the read-only-and-predicate gate above the delegation. `open_dedicated` is the only place that touches the dialer; `DedicatedStreamFile` is a thin `File` forwarder whose only real job is ownership — keeping `_connection` alive so the stream outlives the call. There is no new wire format and no second protocol: a dedicated stream is just a second `RemoteFs` over a second bidi stream, dialed with the same attach name.

The boundary it preserves is the layering boundary. The dedicated-stream decision lives in `wanix-mesh`, the one async/iroh edge; `wanix-9p-client`, `wanix-9p`, and the synchronous 9P core never learn that streams can be multiplied. The blueprint flagged this control-plane / data-plane split as the next shape after the single-connection first slice, and the `Arc<Mutex<P9Conn>>` boundary was left as deliberately the only thing that would change (`docs/mesh-the-missing-half-of-9p.md:300-308`). `StreamingImportFs` is that split landing for blocking reads, with the `FileSystem` surface above it left exactly as it was.

## See also

- [9P over iroh QUIC](/concepts/9p-over-iroh-quic) — the transport whose bidi streams this wrapper allocates one of per blocking open.
- [Blocking stream EOF contract](/concepts/blocking-stream-eof-contract) — what a never-EOF read like `events` or `recv` actually promises.
- [Import, export, and /n](/concepts/import-export-and-n) — how a remote node becomes a local subtree this fits into.
- [#agent device](/devices/agent) — the source of `events` and `reply`, two of the three default streaming files.
- [#plumb device](/devices/plumb) — the source of `recv`, the third.
- [The single-frame serve caveat](/concepts/single-frame-serve-caveat) — the served-side analogue this is the mesh-side answer to.

## Status / honest limits

- **Read opens only.** A dedicated stream is dialed only for a *read* open that matches the predicate. Write opens and every ordinary file still ride the shared serial import — by design, since they never park the stream (`crates/wanix-mesh/src/streaming.rs:98-106`).
- **The streaming set is a default, not a universal rule.** Out of the box only `#agent/<id>/events`, `#agent/<id>/reply`, and `#plumb/<topic>/recv` get their own stream (`crates/wanix-mesh/src/streaming/predicate.rs:20-52`). Any other never-EOF service file you import would block the shared connection unless you pass a custom `StreamPredicate`.
- **This fixes the import side, not the export side.** `StreamingImportFs` keeps blocking imported reads from freezing each other across the mesh. The *served* side still handles one 9P frame at a time per connection, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on that same served connection — that is a separate caveat, addressed where it lives.
- **The mesh exec/agent plane is local-trust only.** The `#agent` streams this routes are part of the local-trust exec surface; the mesh trust boundary still gates who may attach at all, and these conveniences do not change that the exec devices are not for arbitrary untrusted peers.
