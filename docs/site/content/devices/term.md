---
title: "#term — The Terminal Device"
slug: devices/term
pageType: device
oneLiner: "Plan 9-style terminal service: reading new allocates a resource exposing id/ctl/data/program/winch; data and program cross-feed (with \\n→\\r\\n), and winch broadcasts 'cols rows' to readers."
audience: [developer]
tags: [shipped, cli, device, terminal]
sourceRefs:
  - crates/wanix-term/src/lib.rs:75-114
  - crates/wanix-term/src/files/stream.rs:43-91
  - crates/wanix-term/src/files/winch.rs:17-107
  - crates/wanix-term/src/files.rs:151-177
  - crates/wanix-term/src/path.rs:15-30
  - crates/wanix-term/src/state.rs:30-49
  - docs/adrs/0003-terminal-device-and-shell-lifecycle.md:21-46
seeAlso:
  - concepts/qjs-shell
  - concepts/raw-vs-cooked-input
  - concepts/resize-winch-lifecycle
  - concepts/blocking-stream-eof-contract
  - concepts/service-devices
  - devices/index
prerequisites:
  - concepts/service-devices
usedInFlows: []
honestLimits:
  - "ctl accepts only close; there is no signal, kill, or process-group command — close is resource cleanup, not task cancellation."
  - "Resize wakeups are not yet independent of stdin: a guest learns about a winch by reading the winch file, not via an out-of-band interrupt."
  - "The crossfeed queues are unbounded in-memory VecDeques; there is no backpressure or flow control on a stuck reader."
---

# #term — The Terminal Device

Plan 9-style terminal service: reading `new` allocates a resource exposing `id`/`ctl`/`data`/`program`/`winch`; `data` and `program` cross-feed (with `\n`→`\r\n`), and `winch` broadcasts `'cols rows'` to readers.

## What & why

A terminal is not a special kernel object in Wanix — it is a directory of files. Reading `#term/new` hands you a resource id; under that id sit five files that, between them, are an entire terminal: one byte stream for the human/client side, one for the program side, a resize channel, an id readback, and a control file. A task that wants a terminal binds its fds 0/1/2 to the program side; whatever is driving the screen — a native shell loop, the browser cockpit, a future v86 console — attaches to the data side. Because `#term` is a plain `FileSystem` (`crates/wanix-term/src/lib.rs:116`), the same device shape works over native fds, over served 9P, and across the mesh, and clients never invent a process-specific terminal API. ADR 0003 fixes this contract (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:21-46`).

## `#term/new` — allocate a resource

Show it first. Reading the file `new` returns a freshly minted resource id followed by a newline:

```text
$ read #term/new
1
```

Each read of `new` calls `alloc`, which bumps an internal counter with `saturating_add(1)` (so the first id is `1`), inserts a fresh `TermResource`, and returns the id as `<id>\n` (`crates/wanix-term/src/lib.rs:75-85`, `crates/wanix-term/src/files.rs:87-100`). `new` is read-only; opening it for write is rejected (`require_read_only`, `crates/wanix-term/src/files.rs:13-21`). This is the Plan 9 *clone* idiom: reading a well-known file mints a new resource and tells you its name.

## `#term/<id>/id` — report the resource id

`<id>/id` is a tiny read-only file that echoes the resource id as `<id>\n` (`crates/wanix-term/src/open.rs:32-38`). It exists so a client that opened a resource by some other route can read back which id it is holding — useful when the allocating read and the using code are in different layers.

## `#term/<id>/data` — the terminal/client side

`data` is the end a human (or a frontend) holds. Writing to `data` enqueues bytes onto `data_to_program`; reading from `data` drains `program_to_data` (`crates/wanix-term/src/files/stream.rs:43-59`, `36-41`). So keystrokes typed into `data` become input the program reads, and program output appears when you read `data`. The file mode is `0o666` — readable and writable (`crates/wanix-term/src/lib.rs:29`).

## `#term/<id>/program` — the program/task side

`program` is the mirror image, and it is where a task binds fds 0/1/2 (ADR 0003, `docs/adrs/0003-terminal-device-and-shell-lifecycle.md:41-43`). Writes to `program` are program *output*: they land on `program_to_data` (so the client reading `data` sees them), and a lone `\n` is rewritten to `\r\n` on the way through, unless the previous byte already was `\r` (`crates/wanix-term/src/files/stream.rs:50-59`, `80-91`). Reads from `program` drain `data_to_program` — the bytes the client wrote to `data`. That single `\n`→`\r\n` rule is the only cooking the device itself does; everything else (echo, line editing, Ctrl-C cancellation, Ctrl-D exit) belongs to the guest shell, per [raw vs. cooked input](/concepts/raw-vs-cooked-input).

The crossfeed, stated once: **write `data` → read `program`** (input), and **write `program` → read `data`** (output, CRLF-normalized).

## `#term/<id>/winch` — resize broadcast

`winch` carries window-size changes as text. The driver writes a payload like `"80 24\n"` (columns, rows), and every open reader receives it. The mechanics are a fan-out: opening `winch` for read registers a subscriber with its own queue (`crates/wanix-term/src/files/winch.rs:17-41`); a write copies the bytes into *every* subscriber's queue (`crates/wanix-term/src/files/winch.rs:61-75`); and dropping the reader removes its subscriber so the broadcast set stays tight (`crates/wanix-term/src/files/winch.rs:99-107`). A reader opened write-only gets `PermissionDenied` on read, and a non-writable handle is refused on write — the open flags decide direction (`crates/wanix-term/src/files/winch.rs:44-48`, `61-63`). ADR 0003 pins the payload format as `columns rows\n` (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:31-32`). See [resize / winch lifecycle](/concepts/resize-winch-lifecycle) for how a session threads these notifications.

## `#term/<id>/ctl` — the one command is `close`

`ctl` is a control file, and its entire vocabulary is one word. Writing accumulates bytes, trims them, and matches against `close`; a partial prefix is accepted-but-buffered, and only the exact string `close` actually fires (`crates/wanix-term/src/files.rs:157-172`). On `close`, the device removes the resource from the table (`crates/wanix-term/src/lib.rs:96-105`). Anything that is not a prefix of `close` returns `NotSupported`.

After close, existing handles do not silently keep working: every stream and winch operation first calls `ensure_open`, which returns `InvalidFd` once the resource is closed (`crates/wanix-term/src/state.rs:39-49`, `crates/wanix-term/src/files/stream.rs:30`, `65`). A client that *owns* a terminal should write `close` to `ctl` when it is done — but read ADR 0003 carefully: this is **resource cleanup, not task signalling**. There is no kill, no SIGWINCH-as-signal, no process group. Closing the terminal frees the device resource; the task it fed lives or dies by its own exit state (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:44-46`).

## Queue-aware read readiness

The terminal streams override `read_ready` instead of defaulting to always-readable. A `data`/`program` handle reports readable only when its source queue is non-empty (`crates/wanix-term/src/files/stream.rs:65-77`), and a `winch` reader reports readable only when its own subscriber queue has bytes (`crates/wanix-term/src/files/winch.rs:81-96`). This is what lets a poll loop or a 9P client block correctly: a read of an empty terminal returns `0` rather than spinning, and readiness flips true the moment the other side writes. The bytes themselves move through small FIFO queues (`read_from_queue`, `crates/wanix-term/src/files/queue.rs:5-13`), with output draining tracked per-handle so the `\r` insertion stays correct across writes (`prev_written`, `crates/wanix-term/src/files/stream.rs:80-91`).

## See also

- [#term/qjs-shell](/concepts/qjs-shell) — the guest shell that binds 0/1/2 to a terminal's program side.
- [Raw vs. cooked input](/concepts/raw-vs-cooked-input) — who owns echo, editing, and control bytes (the guest, not the device).
- [Resize / winch lifecycle](/concepts/resize-winch-lifecycle) — threading `winch` notifications through a session.
- [Blocking stream & EOF contract](/concepts/blocking-stream-eof-contract) — how queue-aware readiness composes with other byte-stream devices.
- [Service devices](/concepts/service-devices) — the `#name` pattern `#term` belongs to.
- [Devices index](/devices/index) — the rest of the service device set.

## Status / honest limits

`#term` is shipped and used by native CLI shell demos and the browser cockpit's qjs-backed terminal sessions. The contract is deliberately small, and the edges are real engineering boundaries, not bugs:

- **`ctl` accepts only `close`.** There is no signal, kill, or process-group command (`crates/wanix-term/src/files.rs:164-171`). `close` is resource cleanup; it does not cancel the task that was bound to the program side (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:44-46`). A frontend translating Ctrl-C into byte `0x03` is sending terminal *input*, not invoking a Wanix cancellation primitive.
- **Resize wakeups are not yet stdin-independent.** A guest discovers a window-size change by reading `winch`; the device does not interrupt a guest that is blocked elsewhere. Truly out-of-band resize delivery is queued work in the broader shell lifecycle.
- **The crossfeed queues are unbounded in-memory `VecDeque`s** (`crates/wanix-term/src/state.rs:52-62`). A reader that never drains will let the other side's writes accumulate without backpressure; there is no flow control. State lives only as long as the resource — there is no persistence and none is intended.
