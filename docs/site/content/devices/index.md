---
title: Devices
slug: devices/index
pageType: device
oneLiner: Every service device at a glance, with a per-file anchor table for each.
audience: [newcomer, developer, visionary]
tags: [devices, service-devices, shipped, local-trust-only, mesh, cli]
sourceRefs:
  - crates/wanix-task/src/lib.rs
  - crates/wanix-term/src/lib.rs:1-31
  - crates/wanix-kv/src/lib.rs:1-29
  - crates/wanix-pipe/src/lib.rs:1-34
  - crates/wanix-plumb/src/lib.rs:1-47
  - crates/wanix-cas/src/lib.rs:1-43
  - crates/wanix-agent/src/lib.rs:1-46
  - crates/wanix-cpu/src/lib.rs:1-51
  - crates/wanix-cli/src/serve/roots.rs:119-120
seeAlso:
  - concepts/service-devices
  - concepts/devices-import-for-free
  - concepts/everything-is-a-file
  - devices/task
  - devices/term
  - devices/kv
  - devices/pipe
  - devices/plumb
  - devices/cas
  - devices/agent
  - devices/cpu
prerequisites: []
usedInFlows: []
honestLimits:
  - "Exec devices (#task/#agent/#cpu) are local-trust-only; not exposed to untrusted/public peers, and there are no hard CPU/memory limits yet."
  - "#kv is in-memory: state lives only as long as the serve process unless you freeze a capsule."
  - "The served #agent runs a deterministic FakeEngine, not a live LLM; real codex is the local-trust 'wanix agent' CLI path only."
canonicalCaveatFor: []
---

# Devices

Every service device at a glance, with a per-file anchor table for each.

A Wanix "device" is not a kernel driver. It is a plain `FileSystem` bound into
the namespace under a `#name`, so the way you drive it is the way you drive any
file: open, read, write, list. Allocate a process by reading `#task/new/qjs`.
Set a value by writing `#kv/config`. Send a message by writing
`#plumb/build/send`. Because each device is just a filesystem, the same code
that reaches a local device reaches a remote one, and the device's whole
contract is a directory tree you can `ls`. This page is the map: the eight
devices, their files, and which of them cross the mesh for free.

## The `#name` convention

Each device is a `FileSystem` value — `KvDevice`, `PipeDevice`, `TermDevice`,
and so on — bound into a namespace at a `#name` prefix
(`crates/wanix-kv/src/lib.rs:1-29`, `crates/wanix-term/src/lib.rs:1-31`). There
is no device table, no `ioctl`, no special syscall surface: a device only ever
sees `open`/`read_dir`/`metadata`/`remove_file` calls, exactly like a directory
on disk. The `#` prefix is a Plan 9 convention marking a kernel-provided device
namespace; in Wanix it marks a service the runtime binds in for you. The set
`serve --wanix-services` exposes for inspection is a single source of truth —
`#task`, `#term`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#agent`
(`crates/wanix-cli/src/serve/roots.rs:119-120`); `#cpu` is the mesh exec plane
and lives on the network edge rather than in that inspectable served set.

## The eight devices at a glance

| Device | Crate | What it is | Crosses the mesh for free | Trust |
|---|---|---|---|---|
| [`#task`](/devices/task) | `wanix-task` | Allocate, configure, start, observe processes | exec is local | local-trust exec |
| [`#term`](/devices/term) | `wanix-term` | Terminal resources for shells and TTY-backed tasks | yes (file ops) | safe |
| [`#kv`](/devices/kv) | `wanix-kv` | In-memory key/value store, one file per key | yes | safe, ephemeral |
| [`#pipe`](/devices/pipe) | `wanix-pipe` | In-memory byte channels with EOF on last-writer drop | yes | safe |
| [`#plumb`](/devices/plumb) | `wanix-plumb` | Best-effort pub/sub bus, one topic per directory | yes (gossip) | safe |
| [`#cas`](/devices/cas) | `wanix-cas` | Content-addressed blob store (venti), the data plane | yes | safe |
| [`#agent`](/devices/agent) | `wanix-agent` | An LLM session as files | exec is local | local-trust exec |
| [`#cpu`](/devices/cpu) | `wanix-cpu` | Run a task on a peer against your reverse-exported files | it *is* the mesh | local-trust exec |

## Per-file anchor tables

**`#task`** — process as a directory
(`crates/wanix-task/src/lib.rs`). Read `new/<kind>` to allocate; configure and
run through per-task files.

| Path | Read | Write |
|---|---|---|
| `new/<kind>` | allocate a task, returns its id | denied |
| `<id>/cmd` `env` `dir` | inspect config | configure argv / env / cwd |
| `<id>/ctl` | EOF | verbs: `bind <src> fd/<n>`, `start` |
| `<id>/exit` | observe exit status | driver records status |
| `<id>/fd/<n>` | read the bound fd | write the bound fd |
| `self` | the calling task's own view | — |

**`#term`** — terminal resources
(`crates/wanix-term/src/lib.rs:116-181`). Read `new` to allocate a resource.

| Path | Read | Write |
|---|---|---|
| `new` | allocate a resource, returns its id | denied |
| `<id>/data` | bytes from the terminal | bytes to the terminal |
| `<id>/program` | guest end of the TTY (binds fd 0/1/2) | guest end of the TTY |
| `<id>/ctl` | control | verbs incl. `close` |
| `<id>/winch` | queued resize events | — |
| `<id>/id` | the id | — |

**`#kv`** — smallest database
(`crates/wanix-kv/src/lib.rs:120-166`). One file per key; the directory listing
*is* the key set.

| Path | Read | Write |
|---|---|---|
| `<key>` | the value bytes | set the value (creates on open) |
| `<key>` (remove) | — | `rm` deletes the key |
| `.` (listing) | enumerate all keys | — |

**`#pipe`** — byte channels
(`crates/wanix-pipe/src/lib.rs:100-156`). Read `new` to allocate a channel.

| Path | Read | Write |
|---|---|---|
| `new` | allocate a channel, returns its id | denied |
| `<id>/data` | read end (EOF when all writers drop) | write end |
| `<id>/id` | the id | — |

**`#plumb`** — pub/sub bus
(`crates/wanix-plumb/src/lib.rs:120-166`). A topic directory exists on demand;
no allocation step.

| Path | Read | Write |
|---|---|---|
| `<topic>/send` | denied | publish one newline-JSON envelope |
| `<topic>/recv` | drain envelopes received since open | denied |

**`#cas`** — content-addressed store
(`crates/wanix-cas/src/device.rs`). Blobs keyed by BLAKE3 hash, reads
end-to-end verified.

| Path | Read | Write |
|---|---|---|
| `ingest` | hash of the most-recently-ingested blob | store bytes, then read back the hash |
| `<hash>` | the blob bytes (verified) | — |
| `have/<hash>` | `1\n` if present | — |

**`#agent`** — an LLM session as files
(`crates/wanix-agent/src/lib.rs:132-231`). Read `new` to allocate a session.

| Path | Read | Write |
|---|---|---|
| `new` | allocate a session, returns its id | denied |
| `<id>/prompt` | — | send a prompt |
| `<id>/events` | streaming reply events | — |
| `<id>/pending` | parked approval requests | — |
| `<id>/reply` | wait for and read the reply | — |
| `<id>/status` | session status | — |
| `<id>/ctl` | control | verbs incl. approval resolution, `close` |

**`#cpu`** — remote exec, namespace from here
(`crates/wanix-cpu/src/lib.rs:1-51`). Not a static directory: a caller dials a
peer, reverse-exports a scoped sub-namespace, and the acceptor runs a task whose
world *is* that export. The acceptor's `run_job` is byte-for-byte the local
launch — `allocate_root` → bind world → configure → `start` — only the world is
remote (`crates/wanix-cpu/src/lib.rs:25-31`).

## Which devices import across the mesh for free

Every device is a `FileSystem`, and the mesh import half (`RemoteFs`) mounts a
remote namespace as a local one. So `#kv`, `#cas`, `#pipe`, `#plumb`, and
`#term` — all of which are pure file operations — work unchanged on an imported
peer: `/n/<peer>/#kv/<key>` reads that peer's store as ordinary files
(`crates/wanix-plumb/src/lib.rs:13-16`). `#plumb` goes further: the mesh swaps
its `LocalPlumbPort` for a gossip port so a message sent on one node is received
on another. The exec devices are different in kind: `#task`/`#agent`/`#cpu` run
code, so they are local-trust only and are not exposed to untrusted peers. `#cpu`
is itself the cross-node exec plane, but it carries the same local-trust /
grant-allowlisted boundary.

## See also

- [Service devices](/concepts/service-devices) — the pattern: a device is a
  plain `FileSystem` bound at `#name`.
- [Devices import for free](/concepts/devices-import-for-free) — why a
  `FileSystem` device crosses the mesh without device-specific code.
- [Everything is a file](/concepts/everything-is-a-file) — the principle every
  device on this page realizes.
- Each device page: [`#task`](/devices/task), [`#term`](/devices/term),
  [`#kv`](/devices/kv), [`#pipe`](/devices/pipe), [`#plumb`](/devices/plumb),
  [`#cas`](/devices/cas), [`#agent`](/devices/agent), [`#cpu`](/devices/cpu).

## Status / honest limits

- **Exec devices are local-trust only.** `#task`, `#agent`, and `#cpu` run code
  in-process on the host. They are not exposed to untrusted or public peers, and
  there are no hard CPU or memory limits yet
  (`crates/wanix-cpu/src/lib.rs:43-50`). Think cheap, scalable isolation — not
  safe for arbitrary untrusted code.
- **`#kv` is in-memory.** State lives only as long as the `serve` process; to
  persist, freeze the world into a `.wcap` capsule via `#cas`
  (`crates/wanix-kv/src/lib.rs:1-7`).
- **The served `#agent` is a deterministic `FakeEngine`, not a live LLM.** Real
  codex is the local-trust `wanix agent` CLI path only
  (`crates/wanix-agent/src/lib.rs:1-9`).
- **`#plumb` delivery is best-effort.** A reader that was not subscribed when a
  message was sent never sees it; there is no acknowledgement and no durable
  queue (`crates/wanix-plumb/src/lib.rs:1-9`). And because `serve` handles one
  9P frame at a time per connection, a blocking `recv` cannot interleave with a
  write on the same connection — live pub/sub needs a second connection.
