---
title: Live-Stream vs One-Shot 9P Access
slug: concepts/live-stream-vs-one-shot
pageType: concept
oneLiner: The cockpit's p9.ts routes #term/#pipe/#plumb/#agent stream paths through walk+open at offset 0 instead of one-shot readFile/writeFile, because allocator-owned files reject create and subscriptions must not drain to EOF.
audience: [developer]
tags: [cockpit, mesh, shipped, caveat]
sourceRefs:
  - workbench/src/wanix/p9.ts:143-173
  - workbench/src/wanix/p9.ts:229-248
  - workbench/src/wanix/p9.ts:338-441
  - workbench/src/wanix/p9.ts:450-471
  - workbench/src/web/service-inspector.ts:182-387
seeAlso:
  - concepts/browser-cockpit
  - concepts/direct-9p-operator-surface
  - concepts/blocking-stream-eof-contract
  - concepts/streaming-import-fs
prerequisites:
  - concepts/browser-cockpit
  - concepts/blocking-stream-eof-contract
usedInFlows: []
honestLimits:
  - "A live recv read and a send write on the same path can't be interleaved on one connection: serve handles one 9P frame at a time per connection."
  - "openLiveReadable polls with a 25ms sleep between empty reads; it is liveness, not push — there is no readiness wakeup on the client side."
  - "The classifier in service-inspector is heuristic per device; an unknown #device falls back to linking only id/status/kind/exit."
---

# Live-Stream vs One-Shot 9P Access

The cockpit's `p9.ts` routes `#term`/`#pipe`/`#plumb`/`#agent` stream paths through walk+open at offset 0 instead of one-shot `readFile`/`writeFile`, because allocator-owned files reject create and subscriptions must not drain to EOF.

## What and why

Most files the cockpit touches are ordinary: a config blob, a source file, a `#kv` value. For those, "read the whole thing" and "write the whole thing" are exactly right. But a terminal, a pipe, a plumber topic, and an agent's event feed are not blobs — they are live byte streams that never reach a natural EOF and that the device *mints* on open. Hand one of those to the same `readFile` that works for a config file and you either block forever or silently allocate a resource. So `p9.ts` keeps two code paths and a one-function decision (`isLiveServiceStream`) about which path a given name takes.

## The fork: isLiveServiceStream

`openReadable` and `openWritable` both begin with the same branch (`workbench/src/wanix/p9.ts:338-373`):

```ts
async openReadable(name: string): Promise<ReadableStream<Uint8Array>> {
  if (isLiveServiceStream(name)) {
    return this.openLiveReadable(name);
  }
  const data = await this.readFile(name);   // one-shot: drain to EOF, then close
  // ...wrap data in a one-chunk ReadableStream
}
```

`isLiveServiceStream` is a pure path classifier (`workbench/src/wanix/p9.ts:456-471`). It normalizes the path and matches a small, explicit set:

- `/#term/...` — any terminal stream path
- `/#pipe/<id>/data` — the unidirectional byte channel
- `/#plumb/<topic>/send` and `/#plumb/<topic>/recv` — publish and subscribe
- `/#agent/<id>/events` — the agent's turn-event feed

Everything else — a `#kv` value, an editor file, an agent's `status` snapshot — falls through to the one-shot path. Show, then name: the classifier is the client-side restatement of Plan 9's device convention, where some files are *streams* and most files are *snapshots*.

## The streaming path: walk, open, read at offset 0, forever

`openLiveReadable` walks to the existing fid, opens it `O_RDONLY`, and loops reading at offset 0 (`workbench/src/wanix/p9.ts:388-418`):

```ts
while (!closed) {
  const chunk = await session.read(fid, 0, READ_CHUNK_SIZE);
  if (chunk.length > 0) {
    controller.enqueue(chunk);
  } else {
    await delay(25);   // empty read != EOF; the stream is just quiet
  }
}
```

Note the contract: an empty read is *not* end-of-stream, it is "no bytes right now." The loop never treats a zero-length read as a reason to close; it polls again after 25ms and keeps the fid open until the consumer cancels. Offset stays pinned at 0 because these device files are not seekable byte arrays — each read returns the next available bytes, not a window into a fixed buffer.

`openLiveWritable` is the mirror image (`workbench/src/wanix/p9.ts:420-439`). It opens `O_WRONLY`, not `O_RDWR`:

```ts
// Plan 9 pipe (#pipe) and plumber (#plumb) ends are strictly
// unidirectional and reject O_RDWR; #term tolerates a write-only
// open, so O_WRONLY is the safe common mode for every live writable.
await this.session.open(fid, O_WRONLY);
```

A `#pipe` end is one direction only; `#plumb/<topic>/send` is a publish sink. Asking for read+write would be refused. `#term/<id>/program` tolerates write-only, so `O_WRONLY` is the one mode that opens every live writable without negotiation.

## Why the one-shot helpers are wrong for these paths

The danger isn't subtle, and it cuts both ways.

On reads, `readFile` loops until a chunk comes back short or empty, then closes the fid (`workbench/src/wanix/p9.ts:143-173`). Point that at `#plumb/<topic>/recv` or `#agent/<id>/events` and it never terminates — there is no short chunk on a live subscription, so the await blocks indefinitely.

On writes, `writeFile` first tries to truncate-and-write an existing file, and on failure falls back to issuing a `Tlcreate` against the parent directory (`workbench/src/wanix/p9.ts:229-248`). But `#pipe/new`, `#agent/new`, and the topic and channel directories are *allocator-owned*: you do not create children there, the device mints them. A stray create is either refused or — worse — interpreted as an allocation. The streaming path sidesteps both: it walks to the fid that already exists and writes at offset 0, never touching create.

The service inspector encodes the same taxonomy a layer up (`workbench/src/web/service-inspector.ts:182-387`). It refuses to render a clickable link for ALLOCATOR files (`#agent/new`, `#pipe/new`, `#cas/ingest`), for CONTROL files (`ctl`, write-only), and for STREAM files (`events`, `data`, `send`, `recv`, `program`, `winch`) — "opening these can consume live data or block indefinitely." Only METADATA snapshots (`id`, `status`, `kind`, `exit`) link as plain reads. Same boundary, drawn twice: once where bytes move, once where a human might click.

## See also

- [Browser cockpit](/concepts/browser-cockpit) — the operator surface `p9.ts` backs.
- [Direct-9P operator surface](/concepts/direct-9p-operator-surface) — how the cockpit speaks 9P with no bridge.
- [Blocking stream EOF contract](/concepts/blocking-stream-eof-contract) — the device-side rule this page mirrors on the client.
- [Streaming import FS](/concepts/streaming-import-fs) — the same "don't drain to EOF" reasoning across the mesh.
- [#pipe device](/devices/pipe), [#plumb device](/devices/plumb), [#term device](/devices/term), [#agent device](/devices/agent) — the streams being opened.

## Status / honest limits

- Live receive and send cannot interleave on one connection. `serve` handles one 9P frame at a time per connection, so a blocking `#plumb/<topic>/recv` read and a `send` write on the same socket cannot overlap — live pub/sub wants a second connection. The cockpit's self-check exercises the publish path for this reason.
- The readable loop polls, it does not push. `openLiveReadable` sleeps 25ms between empty reads (`workbench/src/wanix/p9.ts:402`); there is no client-side readiness wakeup, so latency is bounded by the poll interval, not by arrival.
- The classifier is per-device and heuristic. `isLiveServiceStream` lists exactly the shipped stream paths; an unrecognized `#device` in the inspector falls back to linking only `id`/`status`/`kind`/`exit` and leaving the rest plain (`workbench/src/web/service-inspector.ts:380-387`). New devices must be added to both classifiers by hand.
