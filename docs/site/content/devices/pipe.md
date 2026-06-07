---
title: "#pipe — In-Memory Byte Channels"
slug: devices/pipe
pageType: device
oneLiner: "#pipe/new allocates a channel; <id>/data is a unidirectional read/write end with EOF on last-writer drop, composing tasks like a Unix pipe."
audience: [developer]
tags: [device, mesh, shipped, caveat]
sourceRefs:
  - crates/wanix-pipe/src/lib.rs:1-167
  - crates/wanix-pipe/src/channel.rs:7-104
  - crates/wanix-pipe/src/open.rs:7-44
  - crates/wanix-pipe/src/files.rs:90-145
  - crates/wanix-pipe/src/path.rs:18-32
seeAlso:
  - devices/plumb
  - devices/kv
  - concepts/blocking-stream-eof-contract
  - concepts/service-devices
  - concepts/devices-import-for-free
prerequisites:
  - concepts/service-devices
  - concepts/blocking-stream-eof-contract
usedInFlows: []
honestLimits:
  - "Channels are in-memory: bytes and channel ids live only as long as the serve process, and there is no on-disk durability."
  - "Each handle is one direction only; opening one file for both read and write returns NotSupported."
  - "The 50ms read re-check is a missed-notification backstop, never an EOF signal — Ok(0) means the last writer dropped."
canonicalCaveatFor: []
---

# #pipe — In-Memory Byte Channels

`#pipe/new` allocates a channel; `<id>/data` is a unidirectional read/write end with EOF on last-writer drop, composing tasks like a Unix pipe.

A Unix pipe is two file descriptors and a kernel buffer between them. `#pipe` is the same idea expressed entirely as files: you read one file to get a fresh channel, then two tasks open the channel's `data` file — one for reading, one for writing — and bytes flow from writer to reader. Because the device is a plain `FileSystem` over `wanix-fs` (`crates/wanix-pipe/src/lib.rs:100-156`), the same channel composes locally between two tasks, across a namespace bind, or across the mesh, with no pipe-specific networking. This page is the contract for that file shape.

## Allocate a channel: read `#pipe/new`

Show it first. Reading the `new` file yields a channel id and a newline:

```text
$ read #pipe/new
3
```

The allocation happens lazily, on the *first read* of the handle: `NewPipeFile::read` calls `device.alloc()` only when its buffer is still empty, then serves back `"<id>\n"` (`crates/wanix-pipe/src/files.rs:44-57`). `alloc` bumps a monotonic counter and inserts a fresh `PipeChannel` into the device's channel map (`crates/wanix-pipe/src/lib.rs:78-89`). The id is a decimal string (`"1"`, `"2"`, `"3"`, …) — stable for the life of the serve process. `new` is read-only: opening it for write, create, or truncate is rejected by `require_read_only` (`crates/wanix-pipe/src/files.rs:8-16`).

Now name it: this is the Plan 9 `clone` pattern. Reading a control file mints a new resource and tells you its name, exactly as `#term/new` and `#task/new` do — see [service devices](/concepts/service-devices).

## Inspect a channel: `#pipe/<id>/id`

Each allocated channel is a directory containing two files, `data` and `id` (`crates/wanix-pipe/src/lib.rs:143-152`). The `id` file is a fixed, read-only snapshot of the channel id — handy for confirming which channel a handle refers to without re-reading `new`:

```text
$ cat #pipe/3/id
3
```

Opening `<id>/id` resolves the channel (a missing channel is `NotFound`) and returns a `BytesFile` holding `"<id>\n"` (`crates/wanix-pipe/src/open.rs:18-22`). Like `new`, it is read-only.

## The data stream: one direction per handle

`#pipe/<id>/data` is where the bytes move. The rule that makes it pipe-shaped instead of file-shaped: **a single open is exactly one direction.** `open_data` enforces it (`crates/wanix-pipe/src/open.rs:27-44`):

- `create` or `truncate` on `data` → `PermissionDenied`. You cannot grow or reset a pipe; it is a stream, not a file.
- `read && write` on one handle → `NotSupported`. A handle is a reader *or* a writer, never both.
- `read` only → a `PipeReader` over the channel.
- `write` only → a `PipeWriter`, which registers itself as a writer on open.
- neither → `PermissionDenied`.

Two tasks compose by each opening the same `data` path with opposite intent:

```text
# producer
$ echo hello | write #pipe/3/data
# consumer (blocks until the producer writes, then drains)
$ read #pipe/3/data
hello
```

Multiple writers are allowed; the channel tracks a writer count, not a single owner.

## Write end: append, then signal EOF on drop

A `PipeWriter` calls `channel.add_writer()` when it is opened, incrementing the channel's writer count (`crates/wanix-pipe/src/files.rs:120-125`, `crates/wanix-pipe/src/channel.rs:46-50`). Each write appends to the channel's `VecDeque<u8>` and wakes any blocked reader with `notify_all` (`crates/wanix-pipe/src/channel.rs:62-69`). Reading the write end is `PermissionDenied` — it is write-only (`crates/wanix-pipe/src/files.rs:128-130`).

The important behavior is in `Drop`: when the writer handle is dropped, `close_writer` decrements the writer count and notifies waiters (`crates/wanix-pipe/src/files.rs:141-145`, `crates/wanix-pipe/src/channel.rs:54-59`). That drop is the *only* thing that can eventually produce EOF. Closing your write end is how you tell the reader "no more bytes are coming."

## Read end: drain, block, EOF only at the end

A `PipeReader` delegates to `channel.read`, whose loop is the heart of the device (`crates/wanix-pipe/src/channel.rs:75-96`):

1. If buffered bytes exist, copy up to `buf.len()` of them out and return that count.
2. Else if the writer count is zero, return `Ok(0)` — this is EOF.
3. Else wait on the condvar with a 50ms timeout, then loop.

The 50ms `READ_RECHECK_INTERVAL` is deliberately *not* an EOF signal. Its own doc comment is emphatic: a write always `notify_all`s, so the timeout only bounds the rare missed-notification case; returning at a timeout would make an empty-but-open pipe indistinguishable from a closed one, because both look like `Ok(0)` to `fd_read` (`crates/wanix-pipe/src/channel.rs:7-15`). So the loop returns `Ok(0)` *only* when there are no buffered bytes **and** the last writer has dropped. This is the [blocking-stream EOF contract](/concepts/blocking-stream-eof-contract): `Ok(0)` is end-of-stream, never "nothing right now." `read_ready` reports the non-blocking cases — bytes buffered, or all writers gone (`crates/wanix-pipe/src/channel.rs:100-103`).

That contract is what lets a downstream task loop `read` until `Ok(0)` and trust that it has seen every byte, the same way a Unix reader trusts a zero-length `read()` on a pipe.

## Meshes for free

Because `#pipe` is just a `FileSystem`, mounting it into another node's namespace gives that node a remote channel with no extra code — see [devices import for free](/concepts/devices-import-for-free). Note the practical limit: live bidirectional streaming over a single served 9P connection is constrained by the [single-frame serve caveat](/concepts/single-frame-serve-caveat) — a blocking read and a write on the *same* connection cannot interleave. For pipes that usually means giving the reader and writer ends separate handles, which the unidirectional rule already encourages.

## See also

- [#plumb — the plumber bus](/devices/plumb) — the message-routing sibling, when you want addressed envelopes instead of a raw byte stream.
- [#kv — the smallest database](/devices/kv) — durable-ish state versus `#pipe`'s ephemeral stream.
- [The blocking-stream EOF contract](/concepts/blocking-stream-eof-contract) — the `Ok(0)`-means-EOF rule `#pipe` shares with other streaming devices.
- [Service devices](/concepts/service-devices) — the `new`/`<id>` clone pattern across `#pipe`, `#term`, and `#task`.
- [Devices import for free](/concepts/devices-import-for-free) — why a plain `FileSystem` reaches across the mesh unchanged.

## Status / honest limits

- **In-memory only.** Channels and their ids live in `DeviceState` for the life of the serve process (`crates/wanix-pipe/src/lib.rs:36-71`); there is no persistence and no replay. When the process exits, every channel and any unread bytes are gone.
- **Unidirectional handles only.** One open is read *or* write. Opening `data` for both directions returns `NotSupported`, and `create`/`truncate` on `data` returns `PermissionDenied` (`crates/wanix-pipe/src/open.rs:27-44`). For two-way communication, allocate two channels or two handles.
- **The 50ms re-check is a backstop, not EOF.** `Ok(0)` from a read means the last writer has dropped and the buffer is empty — full stop. A reader that treats a timeout as end-of-stream will truncate a live pipe (`crates/wanix-pipe/src/channel.rs:7-15`, `75-96`).
