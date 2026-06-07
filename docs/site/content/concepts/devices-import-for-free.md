---
title: Devices Import Across the Mesh for Free
slug: concepts/devices-import-for-free
pageType: concept
oneLiner: Because every service device is a plain FileSystem, /n/<peer>/#kv/<key> and a peer's #agent work through the one 9P client with no special-cased code.
audience: [newcomer, developer, visionary]
tags: [mesh, shipped, local-trust-only, caveat]
sourceRefs:
  - docs/mesh-the-missing-half-of-9p.md:1237-1246
  - docs/mesh-the-missing-half-of-9p.md:1319-1338
  - crates/wanix-kv/src/lib.rs:120-130
  - crates/wanix-cli/src/mount.rs:26-29
  - crates/wanix-mesh/src/streaming.rs:1-97
  - docs/mesh-blueprint.md:65
seeAlso:
  - import-export-and-n
  - service-devices
  - missing-half-of-9p
  - kv-smallest-database
  - send-agent-to-the-data
  - streaming-import-fs
prerequisites:
  - import-export-and-n
  - service-devices
usedInFlows: [{flow: wire-a-mesh, step: 4}]
honestLimits:
  - "The shipped CLI mount binds a single slot /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is a labelled convention, not yet shipped."
  - "Blocking streaming devices (#plumb/<topic>/recv) need a dedicated stream — StreamingImportFs gives each blocking imported open its own connection."
  - "Exec devices (#task/#agent/#cpu) are local-trust only; --wanix-services is refused on the public endpoint."
  - "The served #agent is a deterministic FakeEngine, not a live LLM; real codex is the local-trust wanix agent CLI path."
  - "#kv is in-memory; a peer's keys live only as long as that node's serve process unless frozen to a capsule."
---

# Devices Import Across the Mesh for Free

Because every service device is a plain `FileSystem`, `/n/<peer>/#kv/<key>` and a peer's `#agent` work through the one 9P client with no special-cased code.

You already know two things: every capability in Wanix is a file (`#kv`, `#task`, `#agent` are all `FileSystem`s), and the mesh's import half binds a remote node at `/n/<peer>` so resolutions into that subtree become 9P exchanges. This page is what happens when you multiply them. The product is not "one more feature." It is the absence of features: nobody writes a network client for the key store, nobody adds a `#kv` case to the transport, and a brand-new device you write tomorrow is reachable across every node the moment it exists. The namespace is the integration layer.

## The namespace is the integration layer

Imagine the alternative. If storage shipped a `KvClient`, the task table shipped a `TaskManager`, and agents shipped an `AgentSession`, then making each one reachable over the network would mean three network clients, three serializers, three reconnection stories — and the same again for the next device. Plan 9's escape hatch, restated for Wanix: make services *files*, and one file-transport protocol moves them all. As the blueprint puts it, every remote capability — a peer's files, its `#kv`, its `#task`, a running agent — is "just a `FileSystem` reachable through `Namespace` resolution at `/n/<node>/...`" (`docs/mesh-blueprint.md:65`). The mesh adds exactly one core primitive (a `FileSystem` that speaks 9P to a remote server) and one edge (iroh transport plus identity). Everything else is composition of mechanisms Wanix already had.

## Show it: drive a peer's store with one 9P client

Bring up two nodes. Node A exports its services; node B mounts A and reads A's key store as files. (First build: `cargo build --package wanix-cli; alias wanix-rust='./target/debug/wanix-rust'`.)

```sh
# Node A: export the services namespace on a local-trust direct address.
wanix-rust serve --wanix-services --addr 127.0.0.1:9100

# Node B: bind A at /n/A, then operate A's #kv as ordinary files.
cat '/n/A/#kv/config'                 # read A's value
echo 'done' > '/n/A/#kv/result'       # write A's value
ls   '/n/A/#kv'                        # enumerate A's keys
```

Every line is open, read, write, list — the same four verbs you would use on a local `#kv`. The bytes ride a `Twalk` + `Tlopen` + `Tread`/`Twrite` over the imported `RemoteFs`, over the same QUIC bidi stream that carried regular files (`docs/mesh-the-missing-half-of-9p.md:1319-1338`). The `#kv` device never learns it is on a network; the network never learns it is carrying a key store.

This is also where "send the agent to the data" stops being rhetoric: an agent that operates a key store as `cat`/`write`/`ls` over its *local* `#kv` operates a *remote* node's `#kv` with the identical vocabulary the moment `/n/A` is bound. The data stays put as files on A; the operator works it from B.

## The control experiment

Here is the careful part. The terminal and the task table also crossed the wire in the earliest mesh slices — but those were services the demo author already had in hand, so "it works" could have been an artifact of foreknowledge. `#kv` is the control experiment: a device written with *no thought of the network*, bound into a namespace, and reachable at `/n/A/#kv/<key>` from another machine with not one line of `#kv`-specific code in the client, the transport, or the protocol (`docs/mesh-the-missing-half-of-9p.md:1237-1246`).

The architectural heart of the slice is what it did *not* build: no `KvOverMesh` adapter, no `#kv` case in `RemoteFs`, no `#kv` opcode in the protocol, no `#kv` branch in the QUIC handler. `KvDevice` implements `open`/`metadata`/`read_dir`/`remove_file` (`crates/wanix-kv/src/lib.rs:120-130`) and the served namespace binds it:

```rust
namespace.bind(Arc::new(KvDevice::new()), ".", "#kv", BindOptions::default())?;
```

The bind is the integration. Hold the device as the variable and everything else fixed, and the bytes still cross byte-exact — *that* is the proof that service files cross nodes, not the device.

## The full Plan 9 cast, slice by slice

Each device is a `FileSystem` plus one bind, so each one imports the moment it is served:

- `#kv` — a key is a file; read gets the value, write sets it. A peer's store is `/n/A/#kv/<key>`. See [#kv as the smallest database](/concepts/kv-smallest-database).
- `#pipe` — `#pipe/new` allocates a channel, `<id>/data` is the read/write end. See [#pipe byte channels](/devices/pipe).
- `#plumb` — `<topic>/send` publishes, `<topic>/recv` drains. The plumber bus, imported. See [#plumb plumber bus](/devices/plumb).
- `#cas` — `<hash>` reads a verified blob, `ingest` writes-then-reads-hash. See [#cas content-addressed store](/devices/cas).
- `#agent` — `new`/`prompt`/`events`/`status` as files; an LLM session you can `cat`. See [#agent device](/devices/agent).
- `#cpu` — runs a task against the caller's reverse-exported namespace. See [send the agent to the data](/concepts/send-agent-to-the-data).
- `#task`, `#term` — the task table and terminals; the originals this family is modeled on.

## Why a new device gets all three for free

Write one new device that satisfies the `FileSystem` trait and you get three capabilities at once, with no extra code: it can be **served** over 9P (the server adapter takes any `FileSystem`), it can be **imported** across the mesh (bind a `RemoteFs` at `/n/<peer>` and resolution into it becomes 9P), and it shows up in the **cockpit inspector** (the operator surface drives the served namespace over direct 9P, so any file-shaped device is already inspectable). You wrote a 9P client once; a new file-shaped service becomes reachable across every node the moment it exists.

## See also

- [Import, export, and /n/](/concepts/import-export-and-n) — the placement operation that makes a peer's files local.
- [Service devices (#name)](/concepts/service-devices) — the `#`-named device catalog.
- [The missing half of 9P](/concepts/missing-half-of-9p) — why import, not just export, completes the protocol.
- [#kv as the smallest database](/concepts/kv-smallest-database) — the control-experiment device, up close.
- [Send the agent to the data](/concepts/send-agent-to-the-data) — operate a remote namespace with local verbs.
- [StreamingImportFs](/concepts/streaming-import-fs) — how blocking imported reads get their own stream.

## Status / honest limits

- **The `/n/<peer>` slot is a convention, not yet per-peer.** The shipped CLI mount binds a single fixed slot `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` addressing is designed but unshipped. Read `/n/A` here as a labelled convention.
- **Blocking streaming devices need a dedicated stream.** A `#plumb/<topic>/recv` parks until something arrives, which would freeze the shared import connection. `StreamingImportFs` gives each blocking imported open its own stream (`crates/wanix-mesh/src/streaming.rs:1-97`); ordinary opens still share one. The served side still handles one 9P frame at a time per connection, so a blocking recv cannot interleave with a write on that same served connection.
- **Exec devices are local-trust only.** `--wanix-services` binds `#task`/`#agent`/`#cpu`, which run guest code. The flag is refused on the public endpoint regardless of grants, and allowed only on a direct-address-only `--addr` socket whose ticket is traded out of band. This is cheap, scalable isolation, not a sandbox safe for arbitrary untrusted code; there are no hard CPU/memory limits yet.
- **The served `#agent` is a deterministic FakeEngine,** not a live LLM. Real codex is the local-trust `wanix agent` CLI path only.
- **`#kv` is in-memory.** A peer's keys live only as long as that node's serve process; freeze a world to a capsule to persist.
