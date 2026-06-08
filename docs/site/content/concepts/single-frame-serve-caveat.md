---
title: Single-Frame Serve Caveat (Live recv)
slug: concepts/single-frame-serve-caveat
pageType: concept
oneLiner: The single-connection serve handles one 9P frame at a time, so a blocking #plumb recv cannot interleave with a send on the same connection; live delivery needs a second connection.
audience: [developer]
tags: [serve, mesh, plumb, caveat, shipped, cli]
sourceRefs:
  - crates/wanix-cli/src/p9_ws/connection.rs:59-99
  - workbench/src/web/cockpit-self-check.ts:368-398
  - crates/wanix-mesh/src/streaming.rs:1-24
  - docs/integration/STATUS.md:471-474
  - AGENTS.md:242-247
  - docs/mesh-blueprint.md:143
seeAlso:
  - devices/plumb
  - concepts/serve-composition-surface
  - concepts/streaming-import-fs
  - concepts/browser-cockpit
  - concepts/best-effort-epidemic-delivery
  - concepts/trust-boundary-gaps
prerequisites:
  - devices/plumb
  - concepts/serve-composition-surface
usedInFlows: []
honestLimits:
  - "The serve 9P WebSocket loop processes exactly one frame at a time per connection; there is no per-tag demultiplexer."
  - "A blocking Tread on #plumb/<topic>/recv parks the only connection, so a follow-up send frame on that same connection is never processed."
  - "The cockpit self-check verifies the #plumb publish path only, not end-to-end delivery, because of this constraint."
  - "serve/ has no shutdown signal yet, which is why a concurrency fix is queued rather than landed."
canonicalCaveatFor:
  - single-frame-serve
---

# Single-Frame Serve Caveat (Live recv)

The single-connection serve handles one 9P frame at a time, so a blocking `#plumb` recv cannot interleave with a send on the same connection; live delivery needs a second connection.

## What & why

`#plumb` is a publish/subscribe bus where `#plumb/<topic>/recv` is meant to *block* until an envelope arrives. That works fine when the reader and the writer are different connections. But the Rust `serve` WebSocket connection is strictly serial: it reads one frame, fully handles it, then reads the next. So if a single browser tab issues a blocking read on `recv` and *then* tries to write the matching `send` down the same socket, the `send` frame never gets read — the connection is parked inside the `recv`. The result is a self-deadlock. This page names that boundary, shows exactly where it lives in the code, and explains why it is a sequencing limit rather than a bug.

## See the loop that causes it

The serve WebSocket handler is a plain read-handle-repeat loop (`crates/wanix-cli/src/p9_ws/connection.rs:59-73`):

```rust
loop {
    let Some(message) = read_websocket_message(&mut socket, frames.buffered_len())? else {
        return Ok(());
    };
    if !handle_websocket_message(&mut server, &mut frames, &mut socket, message)? {
        return Ok(());
    }
}
```

One `read`, one `handle`, then back to the top. There is no background reader thread, no per-tag oneshot map, no demultiplexer — the connection is a single in-flight frame at a time. This is a property of the request→response session loop itself, so it is identical across every door over the one session core (the websocket door, the raw-TCP `--p9` door, `p9-stdio`, and the mesh's QUIC stream); the transport refactor under ADR 0006 did not change it. For ordinary 9P traffic (walk, stat, read a `#kv` value, write a file) this is correct and cheap: every request completes promptly, so serial processing is invisible. The mesh import half relies on exactly this property — `RemoteFs` runs one outstanding request behind a `Mutex<P9Conn>` precisely because it is "provably correct against a server that processes one frame at a time" (`docs/mesh-blueprint.md:143`).

## Where it breaks: a blocking Tread that never returns

The contract changes the moment one of those frames is a *blocking* read. `#plumb/<topic>/recv` does not return immediately; it parks server-side until an envelope is published to the topic. On the single serve connection, the `handle_websocket_message` call for that `Tread` does not return, so the loop never reaches its next `read`. Any frame you queue behind it — including the `send` that would unblock it — is stuck in the socket buffer, unread. The reader is waiting on the writer; the writer is waiting on the reader; both share the one channel. That is the deadlock. The same shape applies to any near-never-EOF blocking open, which is why `AGENTS.md:242-247` calls out `#plumb/<topic>/recv` by name as the canonical example.

## Why the cockpit self-check probes publish only

This is not theoretical — it shaped what the [browser cockpit](/concepts/browser-cockpit) actually checks. The "Run Cockpit Self Check" probe deliberately writes one envelope to `#plumb/<topic>/send` and verifies the device *accepts* it, but it does not open a live `recv` subscription on the same connection (`workbench/src/web/cockpit-self-check.ts:368-398`). The code says so flatly: a blocking `recv` on the single browser connection "would prevent the follow-up `send` frame from ever being processed — a self-deadlock." So the self-check confirms the publish path and reports it honestly, rather than dressing up a partial probe as end-to-end delivery (`docs/integration/STATUS.md:471-474`).

## The fix: a second connection, or concurrent frames

The deadlock is purely a sequencing artifact, so there are two clean escapes. The simplest is to put the blocking `recv` on its own 9P connection — a second WebSocket — leaving the first free to publish. The mesh side already does exactly this: `StreamingImportFs` gives each blocking imported open-file its own QUIC bidi stream, so a `Tread` on `#agent/<id>/events`, `#agent/<id>/reply`, or `#plumb/<topic>/recv` stalls only its own stream rather than freezing the whole import (`crates/wanix-mesh/src/streaming.rs:1-24`). See [StreamingImportFs](/concepts/streaming-import-fs) for that pattern. The heavier fix is concurrent frame handling on one connection — a background reader plus a per-tag response map — which the `rpc()` seam is shaped to allow later.

## See also

- [#plumb device](/devices/plumb) — the topic bus whose `recv` triggers this caveat.
- [Serve composition surface](/concepts/serve-composition-surface) — how the single-connection serve is wired.
- [StreamingImportFs](/concepts/streaming-import-fs) — the mesh-side answer to the same blocking-read shape.
- [Browser cockpit](/concepts/browser-cockpit) — the operator surface whose self-check works around this limit.
- [Best-effort epidemic delivery](/concepts/best-effort-epidemic-delivery) — `#plumb` delivery semantics across the mesh.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — other places serve is still maturing.

## Status / honest limits

- The serve 9P WebSocket loop processes exactly one frame at a time per connection; there is no per-tag demultiplexer (`crates/wanix-cli/src/p9_ws/connection.rs:59-73`).
- A blocking `Tread` on `#plumb/<topic>/recv` parks the only connection, so a follow-up `send` frame on that same connection is never read — a self-deadlock, not a slowdown.
- The cockpit self-check verifies the `#plumb` publish path only, not end-to-end delivery, and labels its result accordingly (`workbench/src/web/cockpit-self-check.ts:368-398`).
- `serve/` has no shutdown signal yet, which is why the concurrency fix (a connection cap and concurrent frame handling) is queued rather than landed; the second-connection workaround is available today.
