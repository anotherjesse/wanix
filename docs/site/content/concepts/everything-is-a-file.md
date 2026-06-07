---
title: Everything Is a File
slug: concepts/everything-is-a-file
pageType: concept
oneLiner: In Wanix every capability — storage, tasks, terminals, key/value, agents — is a FileSystem you read, write, and list, so one file protocol (9P) reaches them all.
audience: [newcomer, developer, visionary]
tags: [concept, filesystem, 9p, service-devices, mesh, shipped, caveat, local-trust-only]
sourceRefs:
  - crates/wanix-fs/src/traits.rs:66-334
  - crates/wanix-fs/src/traits.rs:102-130
  - crates/wanix-fs/src/traits.rs:181-205
  - crates/wanix-fs/src/lib.rs:14-31
  - crates/wanix-vfs/src/readdir.rs:101-103
  - crates/wanix-kv/src/lib.rs:1-29
  - crates/wanix-task/src/task_fs.rs
  - crates/wanix-agent/src/path.rs:36-42
  - AGENTS.md:59-60
  - docs/mesh-the-missing-half-of-9p.md:81-89
seeAlso:
  - concepts/the-filesystem-trait
  - concepts/service-devices
  - concepts/per-process-namespaces
  - concepts/devices-import-for-free
prerequisites: []
usedInFlows:
  - {flow: js-outside-chrome, step: 1}
honestLimits:
  - "#kv is an in-memory key/value store, not durable storage; its contents do not survive process exit unless backed elsewhere."
  - "The served 9P websocket handles one frame at a time per connection, so #plumb/<topic>/recv is not yet a clean blocking read on the same connection — live pub/sub needs a second connection or concurrent frame handling."
  - "The served #agent uses a deterministic FakeEngine, not a live LLM; real codex is local-trust-only on the CLI path."
  - "Device-aware semantics (read_ready/write_ready/is_seekable, content_hash) are opt-in; defaults are an ordinary byte file, so a device must override them to expose real readiness or CAS offload."
  - "Mesh import binds a remote namespace at /n/<peer>; attach is capability-gated (default-deny grants), so a peer's devices are not reachable until a grant is in place."
canonicalCaveatFor: []
---

# Everything Is a File

Wanix takes Plan 9's oldest idea literally: a key/value store, a running task, a terminal, an LLM session — each is something you `open`, `read`, `write`, and list as files. There is no separate API for storage and another for processes and a third for agents. There is one trait, `FileSystem`, and everything that wants to be a capability implements it. That single decision is what lets one file-transport protocol (9P) carry *all* of them, locally and — once you have the mesh — across machines.

## A task is a process; a namespace is its file view

Two words show up everywhere in Wanix, so pin them down first.

A **task** is a unit of computation Wanix owns and tracks — a QuickJS script, a compiled `.wasm` program, a shell. It has an id, a command line, file descriptors, and an exit status, and Wanix can list, start, and observe it.

A **namespace** is that task's private view of the file tree. It is not a global filesystem; it is per-process. Two tasks can see different files at the same path, because each one carries its own set of binds. (That mechanism gets its own page — see [per-process namespaces](/concepts/per-process-namespaces).)

The connective tissue between them is the claim in this page's title: the task's *own* identity, its *own* knobs, and every capability it can reach all appear *inside* its namespace, as files.

## Why one trait is the universal currency

If storage exposed a `KvClient`, tasks exposed a `TaskManager`, and agents exposed an `AgentSession`, then making any of them work over the network would mean writing three network clients, three serializers, three reconnection stories. Plan 9's escape hatch was to make services *files* so that one transport — 9P — moves them all. Wanix keeps that contract in Rust as the `FileSystem` trait (`crates/wanix-fs/src/traits.rs`):

```rust
pub trait FileSystem: Send + Sync {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>>;
    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata>;
    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>>;
    // create_dir, remove_file, rename, symlink, set_times, ... — most with
    // a default that returns FsError::NotSupported
}
```

`open` returns a `Box<dyn File>` — a handle with `read`, `write`, `seek`, and `metadata`. A device that only makes sense to read leaves `write` at its default, which returns `NotSupported`; that *is* the device telling you it is read-only. You implement the operations your capability actually has and inherit honest errors for the rest. Anything that satisfies this trait is, by construction, a thing 9P can serve and a namespace can bind.

## Concrete: cat, write, and ls real capabilities

Here is the payoff at the keyboard. Start a JavaScript task — the same script you can run with `wanix qjs examples/qjs-demo.js`:

```js
import * as std from "qjs:std";

// The task's own identity is a file in its namespace.
std.out.puts("task id: " + std.loadFile("#task/self/id").trim() + "\n");

// Storage is a file. Writing #kv/<key> sets the value...
std.writeFile("#kv/greeting", "hello from a Wanix namespace");
// ...reading it back gets it. No client object, just the file.
std.out.puts(std.loadFile("#kv/greeting") + "\n");
```

`#kv` is a plain `FileSystem`: each key is a file, reading `#kv/<key>` returns its value, writing sets it, removing it deletes the key, and listing `#kv` enumerates the keys (`crates/wanix-kv/src/lib.rs`). From a shell over 9P the same capabilities read like ordinary file plumbing:

```sh
# Storage
echo 'on' > '#kv/feature.flag'
cat '#kv/feature.flag'          # -> on

# Tasks: ask what kinds you can start, then read your own exit status
ls '#task/new'                  # -> noop qjs wasm ...
cat '#task/self/id'

# An agent session: allocate one, drive it, watch it
id=$(cat '#agent/new')
echo 'fix the failing test' > "#agent/$id/prompt"
cat "#agent/$id/events"         # streamed turn events
cat "#agent/$id/status"
```

Every line above is the same four verbs — open, read, write, list. The agent's `new`, `prompt`, `events`, `status`, `ctl`, `reply`, and `pending` are just files (`crates/wanix-agent/src/path.rs`); an approval is a write to `ctl`. Nothing here is a special protocol. It is files, the whole way down.

## Under the hood

A few small contracts make the abstraction hold up.

**`FileSystem` + `File`.** The pair in `crates/wanix-fs/src/traits.rs` is the entire surface. `FileSystem` resolves and mutates paths; `File` is an open handle. Most mutating methods default to `FsError::NotSupported`, so a device opts into exactly the operations it supports.

**`NormalizedPath`.** Every method takes a `NormalizedPath`, not a raw string. Paths are normalized once at the boundary — no `..` traversal surprises, no ambiguous separators — so a device implementor never re-parses paths or re-litigates traversal safety.

**`#`-named devices are hidden from union listings.** Service devices live under `#`-prefixed names — `#kv`, `#task`, `#agent`, `#term`, `#pipe`, `#plumb`, `#cas`. The VFS treats a leading `#` as hidden when it composes a directory listing (`crates/wanix-vfs/src/readdir.rs:101`):

```rust
fn is_hidden(name: &str) -> bool {
    name.starts_with('#')
}
```

So `ls /` shows your *files*, not a wall of devices — but the devices are always reachable by name. This is Plan 9's `#`-device convention: addressable, not advertised.

## The payoff: devices import across the mesh for free

Here is where the single trait stops being tidy and starts being load-bearing. Because every device is a *plain* `FileSystem` (`AGENTS.md:59-60`), it is already something 9P can serve and a namespace can bind. The mesh's import half — `RemoteFs` in `wanix-9p-client` — is *also* a `FileSystem`: bind it at `/n/<peer>` and every resolution into that subtree becomes a 9P exchange on the wire.

Nobody had to teach `#kv`, `#agent`, or the task table how to be remote. As the mesh design puts it, you write a 9P client *once* and "every file-shaped service any node exports becomes reachable" (`docs/mesh-the-missing-half-of-9p.md:81-89`). A peer's key/value store is just `/n/A/#kv/<key>`; its agent is `/n/A/#agent/...`. Network transparency falls out of the file abstraction for free, across every device at once. See [devices import for free](/concepts/devices-import-for-free).

## Status and honest limits

"Everything is a file" is the contract, not a magic wand. A few caveats:

- **Device-aware semantics are opt-in.** The `File` trait carries hooks like `read_ready` (does a nonblocking read have data?), `write_ready`, and `is_seekable` — but they have defaults: `read_ready`/`write_ready` default to `Ok(true)` and `is_seekable` to `false` (`crates/wanix-fs/src/traits.rs:102-130`). A device that wants real readiness (a pipe that should not report ready while empty) must override them. The default is "ordinary byte file," and most files happily stay there.
- **`content_hash` is a single, narrow door.** `content_hash` is the *only* hook by which the 9P control plane offloads bulk bytes to the content-addressed data plane (`crates/wanix-fs/src/traits.rs:181-205`). It defaults to `Ok(None)` — "read this the ordinary way" — and only a CAS-backed filesystem overrides it, returning `None` while a file is open for write so a client never fetches a torn snapshot. Above `CONTENT_HASH_OFFLOAD_THRESHOLD` (256 KiB) a CAS-aware client fetches the verified blob peer-to-peer instead of crawling `Tread` windows (`crates/wanix-fs/src/lib.rs:22-31`).
- **Some capabilities are inherently not file-shaped — yet.** Live, blocking pub/sub over a single 9P connection still needs care (the served websocket handles one frame at a time), so `#plumb/<topic>/recv` is not yet a clean blocking read on the same connection. The file *shape* is honest; the *liveness* is still maturing.

The point holds: pick a capability, and the question is not "what API does it expose?" but "what files does it expose, and what do read and write mean there?" That is a much smaller question, and it is the same question every time.

## See also / next

- [The FileSystem trait](/concepts/the-filesystem-trait) — the exact methods, defaults, and what each error means.
- [Service devices](/concepts/service-devices) — the `#`-named device catalog: `#kv`, `#task`, `#term`, `#pipe`, `#plumb`, `#cas`, `#agent`.
- [Per-process namespaces](/concepts/per-process-namespaces) — how each task gets its own private file view.
- [Devices import for free](/concepts/devices-import-for-free) — the mesh payoff: bind a peer at `/n/<peer>` and its devices are just files.
- Next flow: [JS outside Chrome](/learn/js-outside-chrome) — run `wanix qjs examples/qjs-demo.js` and watch a task touch `#task` and `#kv` as files.
