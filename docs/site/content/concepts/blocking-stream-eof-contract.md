---
title: The Blocking-Stream EOF Contract
slug: concepts/blocking-stream-eof-contract
pageType: concept
oneLiner: "#pipe, #plumb recv, and #agent events block until data or true EOF (Ok(0)); a periodic 50ms re-check never falsely signals end-of-stream."
audience: [developer]
tags: [service-devices, streaming, caveat, mesh]
sourceRefs:
  - crates/wanix-pipe/src/channel.rs:7-15
  - crates/wanix-pipe/src/channel.rs:71-104
  - crates/wanix-agent/src/engine.rs:139-166
  - crates/wanix-plumb/src/buffer.rs:16-30
  - crates/wanix-plumb/src/buffer.rs:90-119
seeAlso:
  - devices/pipe
  - devices/plumb
  - devices/agent
  - concepts/single-frame-serve-caveat
  - concepts/streaming-import-fs
prerequisites:
  - concepts/service-devices
usedInFlows: []
honestLimits:
  - "A blocking recv on the served single-frame 9P connection cannot interleave with a write on that same connection; live pub/sub needs a second connection or concurrent frame handling."
  - "#plumb recv buffers per subscription are bounded and lossy: a flooding peer evicts the oldest bytes once the ceiling is hit."
canonicalCaveatFor: [blocking-stream-eof]
---

# The Blocking-Stream EOF Contract

`#pipe`, `#plumb recv`, and `#agent events` block until data or true EOF (`Ok(0)`); a periodic 50ms re-check never falsely signals end-of-stream.

In Wanix a live stream is still just a file you `read`. But "read a file" hides a hard question: when a read returns zero bytes, does that mean *the stream is over* or *nothing has arrived yet*? Get it wrong and a guest that calls `fd_read` on an empty-but-open pipe will think the writer hung up. Three different devices — written by different code paths — answer that question with exactly the same rule, so any guest, shell, or driver can trust `Ok(0)` to mean one thing only: end-of-stream.

## Show it: a read that waits, then a read that ends

Allocate a pipe over 9P and watch a reader block on the empty end while a writer feeds it:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

id=$(cat '#pipe/new')
# Reader blocks here — the pipe is open and empty, so read does NOT return 0.
cat "#pipe/$id/data" &

echo 'first'  > "#pipe/$id/data"   # reader wakes, prints "first"
echo 'second' > "#pipe/$id/data"   # reader wakes, prints "second"
```

The reader prints each line as it arrives and otherwise sits idle. It returns end-of-file only when the last writer end closes — at which point `cat` finally sees `Ok(0)` and exits. An empty, open pipe and a closed pipe are *not* the same thing, even though a naive implementation would report both as zero bytes.

## The shared rule across every live-stream device

The Plan 9 name for this is a **blocking byte stream**: a `read` makes forward progress or it waits, and the only zero-length read is the real end. Wanix encodes that rule identically in three places.

`#pipe`'s channel returns `Ok(0)` only when there is no buffered data *and* every writer end has dropped (`crates/wanix-pipe/src/channel.rs:71-104`):

```rust
if !buffer.data.is_empty() { /* return drained bytes */ }
if buffer.writers == 0 { return Ok(0); }   // EOF: last writer gone
// otherwise: wait on the condvar, then re-check
```

`#agent`'s event stream uses the same loop, gated on a `closed` flag instead of a writer count: `Ok(0)` only when the buffer is empty and the session marked the stream closed (`crates/wanix-agent/src/engine.rs:139-166`). `#plumb`'s per-subscription `recv` buffer is the third copy of the discipline, shared by the in-process `LocalPlumbPort` and the mesh gossip port so both deliver received envelope bytes identically (`crates/wanix-plumb/src/buffer.rs:90-119`).

Each device pairs the read loop with `read_ready()`, which reports `true` when bytes are buffered *or* the stream has ended — both are non-blocking reads. That hook is how the `File` trait surfaces readiness without forcing a blocking call.

## Why the 50ms re-check is a wakeup bound, not an EOF

A reader that simply slept forever would deadlock if it ever missed a notification. So each read waits on a condvar with a worst-case timeout (`crates/wanix-pipe/src/channel.rs:7-15`):

```rust
/// A read blocks until bytes arrive or all writers close, but a write always
/// `notify_all`s, so this timeout only bounds the case where a notification is
/// missed; it is a periodic re-check, NOT an end-of-file signal.
const READ_RECHECK_INTERVAL: Duration = Duration::from_millis(50);
```

Every `write` (and every `close`) calls `notify_all`, so under normal operation a blocked reader wakes the instant data lands — the 50ms is never on the hot path. The timeout exists only to re-acquire the lock and re-check state in the rare event a wakeup is lost. Critically, the wait branch ignores `_timed_out`: a timeout does **not** return. It loops back to the top, re-checks the buffer and the writer/closed flag, and blocks again. The comment spells out the trap it avoids — returning at a timeout "would make an empty-but-open pipe indistinguishable from EOF (both look like `Ok(0)` to `fd_read`)." `#plumb` documents the identical invariant: the re-check "only bounds a missed wakeup; it is never an end-of-stream signal" (`crates/wanix-plumb/src/buffer.rs:16-18`).

## How to implement it correctly in a new device

If you add a live-stream service device, copy the three-part loop and nothing else will surprise a guest:

1. **Hold one lock; loop.** Take the buffer lock and loop. If there are buffered bytes, drain up to `buf.len()` and return that count immediately.
2. **Return `Ok(0)` only at true end.** Track an explicit end condition — a writer count reaching zero, or a `closed` flag. Return `Ok(0)` *only* when the buffer is empty *and* that condition holds.
3. **Wait, do not poll-return.** Otherwise `wait_timeout` on a condvar with a small interval, discard the timed-out flag, and loop. Have every producer call `notify_all` on write and on close so the timeout stays off the hot path.

Mirror it in `read_ready()`: `!data.is_empty() || ended`. That is the entire contract.

## See also

- [The #pipe device](/devices/pipe) — in-memory byte channels; the canonical writer-count EOF.
- [The #plumb device](/devices/plumb) — topic bus whose `recv` reuses this buffer across local and gossip ports.
- [The #agent device](/devices/agent) — the `events` stream that closes when a session ends.
- [Service devices](/concepts/service-devices) — the `#`-named device set this contract lives inside.
- [The single-frame serve caveat](/concepts/single-frame-serve-caveat) — why a blocking recv cannot interleave with a write on one served connection.
- [Streaming import FS](/concepts/streaming-import-fs) — how live reads behave when the file is imported across the mesh.

## Status / honest limits

- **A blocking recv cannot interleave with a write on the same served connection.** The serve 9P transport handles one frame at a time per connection, so a guest that blocks in `#plumb/<topic>/recv` on a connection cannot also publish on it; live pub/sub needs a second 9P connection or concurrent frame handling. The EOF contract is sound; this is a transport limit, not a stream-semantics bug. See [the single-frame serve caveat](/concepts/single-frame-serve-caveat).
- **`#plumb` recv buffers are bounded and intentionally lossy.** Delivery is best-effort epidemic pub/sub, so a per-subscription ceiling drops the oldest bytes once a flooding peer fills it rather than growing without bound (`crates/wanix-plumb/src/buffer.rs:20-30`). A slow reader loses old envelopes; it never sees a false EOF.
- **`#agent` events close when the session closes.** The `events` stream returns `Ok(0)` after the session marks it closed (`crates/wanix-agent/src/engine.rs:131-137`); reopening starts a fresh read, it does not replay drained bytes.
