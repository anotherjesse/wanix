---
title: Add a service device
slug: learn/add-a-service-device
pageType: flow
oneLiner: Implement the FileSystem trait once and get a device that imports across the mesh, exports over 9P, and shows up in the cockpit for free.
audience: [developer]
tags: [extension-point, service-devices, filesystem-trait, mesh, shipped, caveat]
sourceRefs:
  - crates/wanix-kv/src/lib.rs
  - crates/wanix-kv/src/files.rs:70
  - crates/wanix-pipe/src/lib.rs
  - crates/wanix-pipe/src/channel.rs:75
  - crates/wanix-cli/src/serve/roots.rs:187
  - crates/wanix-cli/src/serve/roots.rs:207
  - crates/wanix-fs/src/traits.rs:153
seeAlso:
  - concepts/service-devices
  - concepts/the-filesystem-trait
  - concepts/blocking-stream-eof-contract
  - concepts/devices-import-for-free
  - devices/kv
  - devices/pipe
  - reference/extension-points
prerequisites:
  - learn/js-outside-chrome
usedInFlows: []
honestLimits:
  - "#kv is in-memory; the store lives only as long as the serve process. Freeze a world to a capsule to persist it."
  - "A blocking read on a stream device cannot interleave with a write on the same serve connection — serve handles one 9P frame at a time per connection."
  - "Exec devices (#task, #agent, #cpu) are local-trust only; a plain data device like #kv is not, but adding one does not change the exec trust boundary."
canonicalCaveatFor: []
---

# Add a service device

Implement the `FileSystem` trait once and get a device that imports across the mesh, exports over 9P, and shows up in the cockpit for free.

A Wanix service device is not a special object with its own protocol — it is a plain `FileSystem`. `#kv`, `#pipe`, `#plumb`, and `#cas` are all just crates that implement one trait over `wanix-fs`. Because the mesh carries 9P and every device is a filesystem, the moment your device binds into the served namespace it is reachable, inspectable, and remotely importable with no transport code of your own. This flow walks `#kv` as the canonical minimal device, then `#pipe` for the stream case, then the one bind that makes a device real.

## The extension point

The contract is the `FileSystem` trait in `crates/wanix-fs/src/traits.rs:153`. Almost every method has a default that returns `FsError::NotSupported`, so a device only implements the operations it actually offers. The minimum a readable, listable device implements is `open`, `metadata`, and `read_dir`; a mutable one adds `remove_file`, and a stream device backs `open` with a blocking handle. There is no registry to register with and no `Device` supertrait to satisfy — if it is a `FileSystem`, it is a device.

## #kv: the smallest real device

`#kv` is the worked minimum. The whole device is about 177 non-test lines in `crates/wanix-kv/src/lib.rs`, backed by an `Arc<RwLock<BTreeMap<String, Vec<u8>>>>`. Run it and the shape is obvious before the trait is:

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix-rust='./target/debug/wanix-rust'
mkdir -p project-root                          # serve needs an existing root
wanix-rust serve --wanix-services --listen 127.0.0.1:7654 ./project-root
# in the cockpit or any 9P client: write #kv/greeting=hello, then read it back
```

Each key is a file. Reading `#kv/<key>` returns its value, writing it sets the value, removing it deletes the key, and listing `#kv` enumerates the keys. The path parser is four lines: `.` is the root directory, a bare name is a key, and anything with a `/` is `NotFound` (`crates/wanix-kv/src/lib.rs:109`). That is the entire namespace.

The three trait methods that carry it:

- `open` (line 121) dispatches on `OpenOptions`. A write/create/truncate open eagerly creates the key — `ensure_key` — so a 9P `Tlcreate` that stats the path immediately after open still finds it, then returns a `KvWriteFile`. A read open returns a `KvReadFile` holding a *snapshot* of the bytes taken at open time, so a concurrent overwrite cannot tear an in-flight read.
- `read_dir` (line 142) succeeds only on the root and maps each key to a `DirEntry`.
- `remove_file` (line 156) deletes the key or returns `NotFound`.

The commit semantics live in the `File` handle, not the device. `KvWriteFile` buffers writes and commits the whole buffer to the map in its `Drop` (`crates/wanix-kv/src/files.rs:70`) — close-to-commit, the same write-then-close shape `#cas/ingest` and `#term` use. This is the pattern to copy: keep the `FileSystem` a thin path router, and put per-handle state in the `File`.

## #pipe: the blocking-stream case

A data device is easy; a *stream* device has one rule you must get right. `#pipe` is shaped like `#term`: reading `#pipe/new` allocates a channel and returns its id, and each channel exposes `id` and `data`. Opening `<id>/data` for reading yields the read end, for writing the write end (`crates/wanix-pipe/src/lib.rs`).

The rule is the EOF contract in `crates/wanix-pipe/src/channel.rs:75`. A blocking `read` returns `Ok(0)` — end-of-file — **only** when there is no buffered data *and* every writer handle has been dropped. A read that wakes on a timeout must re-check and keep waiting, never report EOF, because to `fd_read` an empty-but-open stream and a closed one both look like `Ok(0)`. The channel counts writers explicitly: `add_writer` on each write open, `close_writer` (which `notify_all`s any blocked reader) when a write handle drops, and `read` loops until data arrives or `writers == 0`. Get this wrong and a reader either spins or hangs. See [blocking-stream-eof-contract](/concepts/blocking-stream-eof-contract) for the full reasoning.

## Bind it into the served namespace

A device that no one binds is invisible. Two edits in `crates/wanix-cli/src/serve/roots.rs` make it real. First, `bind_host_and_terminal` binds the device at its `#name` (line 207 shows `#kv`):

```rust
namespace.bind(Arc::new(KvDevice::new()), ".", "#kv", BindOptions::default())?;
```

Second, add the name to `INSPECTABLE_SERVICE_DEVICES` at `crates/wanix-cli/src/serve/roots.rs:187` — the single source of truth for which devices discovery advertises and the cockpit lists. The comment there is binding: this set must stay in sync with the actual binds, and the `serve_wanix_services_*` tests exercise each entry over 9P. Adding a name without a bind, or a bind without the name, is the bug those tests catch.

## The payoff: three surfaces, zero extra code

This is why the `FileSystem`-as-device choice pays. Once bound:

- **Mesh import for free.** The mesh carries 9P, so your device is a `FileSystem` on the wire like any other. A peer that imports the namespace reaches it at `/n/<peer>/#yourdevice/...` with no networking code from you (`/n/<peer>` is a labelled convention; see the limits below). This is [devices-import-for-free](/concepts/devices-import-for-free).
- **9P export for free.** `serve` already exports the namespace; your device's `open`/`read_dir`/`metadata` become `Topen`/`Treaddir`/`Tgetattr` responses through the shared synchronous 9P core.
- **Cockpit inspection for free.** Because it is in `INSPECTABLE_SERVICE_DEVICES`, the browser cockpit's device inspector lists and reads it over direct 9P.

Keep tokio and iroh *out* of the device. `wanix-mesh` is the only async edge in the workspace; a service device stays synchronous and engine-free so it composes across every transport.

## See also

- Concept: [service devices](/concepts/service-devices) — the boundary your crate joins.
- Concept: [the FileSystem trait](/concepts/the-filesystem-trait) — the one contract to implement.
- Concept: [the blocking stream / EOF contract](/concepts/blocking-stream-eof-contract) — required reading before a stream device.
- Concept: [devices import for free](/concepts/devices-import-for-free) — why one bind reaches the mesh.
- Devices: [`#kv`](/devices/kv) · [`#pipe`](/devices/pipe) — the two worked examples here.
- Reference: [extension points](/reference/extension-points) — every place the core takes a plug-in.
- Prerequisite flow: [JavaScript outside Chrome](/learn/js-outside-chrome).

## Status / honest limits

- `#kv` is an **in-memory** tier. The `BTreeMap` lives in the serve process and the store vanishes when that process exits (`crates/wanix-kv/src/lib.rs`). For durability, freeze the world into a capsule — `#kv` itself has no on-disk backing today.
- `serve` handles **one 9P frame at a time per connection**. A blocking read on a stream device (a `#pipe` reader, a `#plumb` recv) cannot interleave with a write on that same connection; live pub/sub needs a second connection or concurrent frame handling. This bounds what a single-connection client can do with a blocking device, not the device's correctness.
- Adding a plain data device does **not** move the exec trust boundary. The exec devices — `#task`, `#agent`, `#cpu` — are local-trust only and are not exposed to untrusted peers; the served `#agent` runs a deterministic `FakeEngine`, not a live model. A new data device like `#kv` is just files, but do not read "imports across the mesh for free" as "safe to expose to hostile peers."
- `/n/<peer>` is a **labelled convention** in these examples. The shipped CLI mount binds a single slot `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); the per-peer `/n/<peer-id>` layout is designed but unshipped.
