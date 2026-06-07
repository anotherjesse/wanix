---
title: Service Devices (#name)
slug: concepts/service-devices
pageType: concept
oneLiner: Named "#" devices (#task, #term, #kv, #pipe, #plumb, #cas, #agent) are file trees you operate by read/write/ls instead of bespoke APIs.
audience: [newcomer, developer, visionary]
tags: [devices, namespace, shipped, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-cli/src/serve/roots.rs:115-185
  - crates/wanix-vfs/src/readdir.rs:101-103
  - crates/wanix-wasi/src/ctx.rs:126-141
  - crates/wanix-wasi/src/ctx/path.rs:7-19
  - crates/wanix-cli/src/serve/http/app.rs:40-114
  - AGENTS.md:180-187
seeAlso:
  - concepts/everything-is-a-file
  - concepts/devices-import-for-free
  - devices/task
  - concepts/wanix-services-device-set
  - concepts/blocking-stream-eof-contract
prerequisites:
  - concepts/everything-is-a-file
usedInFlows: []
honestLimits:
  - The served #agent is a deterministic FakeEngine, not a live LLM; the real codex bridge is the local-trust 'wanix agent' CLI path only.
  - "#kv is in-memory: state lives only as long as the serve process unless you freeze it to a capsule."
  - "Exec devices (#task, #agent, #cpu) are local-trust only and are not exposed to untrusted or public peers; there are no hard CPU or memory limits yet."
  - serve handles one 9P frame at a time per connection, so a blocking #plumb recv cannot interleave with a write on the same connection.
---

# Service Devices (#name)

Named "#" devices (`#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`) are file trees you operate by read/write/ls instead of bespoke APIs.

A service device is the place where [everything is a file](/concepts/everything-is-a-file) stops being a slogan and becomes a catalog. Each capability Wanix offers — durable storage, the task table, terminals, byte pipes, a plumber bus, a content store, an agent session — is a `FileSystem` bound under a single `#`-prefixed name. You do not learn seven APIs. You learn `ls`, `cat`, and `echo >`, and the device tells you what its files mean. This page names the shipped set, shows what file operations each cockpit demo really performs, and explains the two small rules that make `#`-devices both addressable and out of your way.

## A device is a FileSystem with a leading # name

Start from the keyboard. Bring up a services-enabled server, then talk to it as files:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
wanix-rust serve --wanix-services

# storage is a file
echo 'on' > '#kv/feature.flag'
cat '#kv/feature.flag'          # -> on

# the task table is a directory
ls '#task/new'                  # -> noop qjs wasm ...
cat '#task/self/id'
```

Each line is one of the four file verbs — open, read, write, list. `#kv` is not a client object with `.get()` and `.put()`; each key *is* a file, and reading or writing the file is reading or writing the value. The leading `#` is the only thing that marks `#kv` as a device rather than a directory in your working tree. That convention comes straight from Plan 9: a `#`-name is a kernel device you address by name, not a path you stumble onto.

## The shipped set, from one source of truth

Wanix does not let the device list drift. The set bound by `serve --wanix-services` is declared once, in `crates/wanix-cli/src/serve/roots.rs:119-120`:

```rust
pub(super) const INSPECTABLE_SERVICE_DEVICES: &[&str] =
    &["#task", "#term", "#kv", "#pipe", "#plumb", "#cas", "#agent"];
```

That constant is both the binding manifest and the discovery advertisement (surfaced to clients as `services.devices`, per `AGENTS.md:180-187`). Each name maps to a plain `FileSystem`:

- **`#task`** — the task table: `#task/new` lists the kinds you can start (`noop`, `qjs`, `wasm`), `#task/self/id` is your own identity. See [the task device](/devices/task).
- **`#term`** — terminal sessions: `#term/new`, then `<id>/ctl`, `<id>/data`, `<id>/program`, `<id>/winch`. See [the term device](/devices/term).
- **`#kv`** — the smallest database: one file per key. See [the kv device](/devices/kv).
- **`#pipe`** — in-memory byte channels: `#pipe/new` allocates, `<id>/data` is the read/write end. See [the pipe device](/devices/pipe).
- **`#plumb`** — the plumber bus: `<topic>/send` publishes, `<topic>/recv` drains. See [the plumb device](/devices/plumb).
- **`#cas`** — the content-addressed data plane: `#cas/<hash>` reads a blob, `#cas/ingest` is write-then-read-hash. See [the cas device](/devices/cas).
- **`#agent`** — an LLM session as files: `new`, `prompt`, `events`, `pending`, `ctl`, `reply`, `status`. See [the agent device](/devices/agent).

The binds happen in `bind_host_and_terminal` and `bind_task_service` (`crates/wanix-cli/src/serve/roots.rs:122-185`); the constant exists precisely so the manifest and the binds cannot disagree.

## What file ops the cockpit demos really are

The [browser cockpit](/concepts/browser-cockpit) drives this set over direct 9P, and every "demo" is plain file plumbing once you look underneath:

- **The agent repair demo** allocates a session by reading `#agent/new`, writes a task to `#agent/<id>/prompt`, reads turn events from `#agent/<id>/events`, and approves a pending action by writing to `ctl` — an [approval is a file](/concepts/approvals-as-files).
- **The qjs to wasm to qjs duet** starts three tasks through `#task/new` against one shared filesystem and watches each exit status — the [two-tiers-one-substrate](/concepts/two-tiers-one-substrate) proof, expressed as reads of `#task`.
- **The HTTP-app demo** serves an app at `/.wanix/app/<name>` (loopback-only, services-gated, shipped on this branch — `crates/wanix-cli/src/serve/http/app.rs:40`) whose counter state is backed by the single key `#kv/http-counter`. The "database" is one file.
- **The self-check** walks `INSPECTABLE_SERVICE_DEVICES` and probes each one over 9P, confirming the device tree is live.

No demo invents a protocol. Each one is `ls`, `read`, and `write` against a `#`-named tree.

## Addressable, not advertised

If `#kv`, `#task`, `#agent`, and four siblings all showed up in `ls /`, your filesystem would be a wall of devices. So the VFS hides any name beginning with `#` when it composes a union directory listing (`crates/wanix-vfs/src/readdir.rs:101-103`):

```rust
fn is_hidden(name: &str) -> bool {
    name.starts_with('#')
}
```

`ls /` shows your files; the devices stay reachable by their exact name but never clutter the listing. This is Plan 9's `#`-device rule in one line: addressable, not advertised.

## Under the hood: binding, and reaching #name from any cwd

Two mechanisms make this work.

**Binding.** `--wanix-services` builds a `Namespace` and binds each device's `FileSystem` at its `#`-name (`crates/wanix-cli/src/serve/roots.rs:127-170`). A bind is the whole mechanism — there is no device registry beyond the namespace itself. Because each device is an ordinary `FileSystem`, binding a *remote* one works identically: that is why [devices import for free](/concepts/devices-import-for-free) across the mesh.

**Reaching `#name` from any cwd.** A WASI guest's working directory is arbitrary, but a `#`-device lives at the namespace root, not relative to the guest's cwd. So path resolution special-cases rooted service names (`crates/wanix-wasi/src/ctx/path.rs:7-19`): if a path's first component is a known `#name`, it resolves from the root instead of being joined to the working directory (`crates/wanix-wasi/src/ctx.rs:137`). A `qjs` or `wasm` task can therefore open `#kv/<key>` no matter where it is `cd`'d to — the device name is an absolute address, not a relative file.

## See also

- [Everything is a file](/concepts/everything-is-a-file) — the trait behind every device.
- [The task device](/devices/task) — the `#task` table in detail.
- [The kv device](/devices/kv), [pipe](/devices/pipe), [plumb](/devices/plumb), [cas](/devices/cas), [agent](/devices/agent) — the rest of the catalog.
- [The wanix-services device set](/concepts/wanix-services-device-set) — what `--wanix-services` binds and advertises.
- [Devices import for free](/concepts/devices-import-for-free) — bind a peer and its `#`-devices are just files at `/n/<peer>`.
- [The blocking stream / EOF contract](/concepts/blocking-stream-eof-contract) — how `#pipe` and `#plumb` signal end-of-stream.

## Status / honest limits

- The served **`#agent` is a deterministic `FakeEngine`**, not a live LLM. The real codex bridge requires auth and unattended execution, so it stays on the local-trust `wanix agent` CLI path (`crates/wanix-cli/src/serve/roots.rs:163-170`).
- **`#kv` is in-memory.** State lives only as long as the serve process. To persist it, freeze the world into a [capsule](/concepts/wanix-capsule).
- **Exec devices (`#task`, `#agent`, `#cpu`) are local-trust only.** They are not exposed to untrusted or public peers, and there are no hard CPU or memory limits yet. Treat them as cheap, scalable isolation, not as safe sandboxing for arbitrary untrusted code.
- **`serve` handles one 9P frame at a time per connection.** A blocking `#plumb/<topic>/recv` cannot interleave with a write on the same connection; live pub/sub needs a second connection or concurrent frame handling.
