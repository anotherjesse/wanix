---
title: The Missing Half of 9P
slug: concepts/missing-half-of-9p
pageType: concept
oneLiner: Wanix could always export a namespace; the mesh adds the 9P client so it can also import one — bind a remote node at /n/<node> and its files and devices become local.
audience: [visionary, developer]
tags: [mesh, plan9, shipped, local-trust-only, caveat]
sourceRefs:
  - docs/mesh-the-missing-half-of-9p.md:1-18
  - docs/mesh-the-missing-half-of-9p.md:63-96
  - docs/mesh-the-missing-half-of-9p.md:451-496
  - crates/wanix-cli/src/mount.rs:1-29
seeAlso:
  - concepts/remotefs-import-half
  - concepts/import-export-and-n
  - concepts/devices-import-for-free
  - concepts/agents-as-operators
  - concepts/wanix-as-host
prerequisites:
  - concepts/per-process-namespaces
  - concepts/the-9p-contract
usedInFlows:
  - {flow: plan9-ideas-tour, step: 4}
honestLimits:
  - "The mesh/agent layer has no ADRs yet; its contracts are fast-moving and captured in design docs, not ratified architecture records."
  - "The shipped CLI mount binds a single slot at /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is designed, not shipped. Use /n/<peer> only as a labelled convention."
  - "Exec devices reachable over an import (#task, #cpu, #agent) are local-trust only; the served #agent is a deterministic FakeEngine, not a live LLM."
---

# The Missing Half of 9P

Wanix could always export a namespace; the mesh adds the 9P client so it can also import one — bind a remote node at `/n/<node>` and its files and devices become local.

For a long time Wanix had a quiet asymmetry. It had per-task namespaces, file-shaped services, programs that are just files you run, and a 9P server that could hand any of that to the network. It could **export**. What it could not do was **import**: take a remote 9P service and splice it into its own tree. It had the 9P *server* and not the 9P *client*. It had half of 9P (`docs/mesh-the-missing-half-of-9p.md:1-18`). This page is about why the other half is the keystone, and why writing it *once* bought network reach across every device at the same time.

## The one-liner, stated as a proof

Plan 9 built network transparency from two operations, not one:

- **export** hands a subtree to the network as a 9P service. Wanix already did this — `wanix-9p` serves any `FileSystem` over a socket.
- **import** takes a remote 9P service and binds it into *your* namespace at a path you choose. This is the half that was missing.

The import half is a single struct, `RemoteFs`, that implements the synchronous `wanix_fs::FileSystem` trait by speaking 9P to a server over a blocking byte stream. It is the exact mirror of the server: where the server decodes T-messages and encodes R-messages, the client encodes T-messages and decodes R-messages. No new wire format, no async runtime, no transport of its own — it turns the existing `wanix-protocol` codecs inside out (`docs/mesh-the-missing-half-of-9p.md:20-29`).

You can watch it work across two processes today. The server exports a host directory as raw 9P over TCP — a `serve` mode, not a separate daemon:

```sh
# build once
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

# Terminal 1 — the server (export)
SROOT=$(mktemp -d)
echo "hello from the server" > "$SROOT/greeting.txt"
wanix serve --root "$SROOT" --p9 127.0.0.1:5640
```

The client imports it and reads, writes, and lists *through the mount* — never a local shortcut. The bytes genuinely traverse `Namespace -> RemoteFs -> 9P -> server` (`docs/mesh-the-missing-half-of-9p.md:440-442`):

```sh
# Terminal 2 — the client (import)
wanix mount-ls    tcp://127.0.0.1:5640
wanix mount-cat   tcp://127.0.0.1:5640 greeting.txt
wanix mount-write tcp://127.0.0.1:5640 docs/note.txt "written across the wire"
```

A file written through the client mount appears in the served directory on the server host. Each verb binds the remote into a fresh namespace and runs one operation through it (`crates/wanix-cli/src/mount.rs:1-9`). That round-trip is the proof: Plan 9 *import*, realized.

## Why "everything is a file" makes this so cheap

Plan 9's deepest idea is not "everything is a file." It is that **the namespace is per-process and you can rearrange it** (`docs/mesh-the-missing-half-of-9p.md:65-67`). The file abstraction is what makes one protocol plus one bind buy network transparency across *every* service at once.

The argument is short. If your services are files, then a single file-transport protocol (9P) plus a single placement operation (bind) gives you network transparency for free, across every service simultaneously. You do not write a network client for the terminal, and another for the task table, and another for the key/value store. You write a 9P client *once*, and every file-shaped service any node exports becomes reachable (`docs/mesh-the-missing-half-of-9p.md:81-89`). A peer's key/value store is just `/n/A/#kv/<key>`; its agent is `/n/A/#agent/...`. Nobody had to teach `#kv` or the task table how to be remote — they were already plain `FileSystem`s, so `RemoteFs` carries them across without modification. See [devices import for free](/concepts/devices-import-for-free).

Wanix already had the *placement* half: `wanix-vfs` has had Plan 9-style bind and namespace resolution from early on. What it lacked was the *transport-import* half — a `FileSystem` you could bind that, when resolved, spoke 9P to somebody else. `RemoteFs` is exactly that object (`docs/mesh-the-missing-half-of-9p.md:90-96`). The convention for where you place it is `/n/`: in Plan 9 you imported a machine's devices under `/n/<name>` — `/n/sources`, `/n/kremvax` — and from then on a remote file was just a path. `cat /n/lab/dev/mouse` read a mouse on another machine. No new API, no client library per service.

## For a long time, Wanix had only half of 9P

It is worth dwelling on the asymmetry because it explains the shape of the whole mesh roadmap. Export without import is a one-way mirror: you can show your world to others, but you cannot reach into theirs. A server that can only be talked *to* is a service, not a peer. The day Wanix gained `RemoteFs`, it stopped being a thing you connect to and became a thing that connects — a node that can compose other nodes into its own tree. Every later mesh slice is incremental precisely because it all rides this one import: it lands as one more byte stream above a core that never learned its name (`docs/mesh-the-missing-half-of-9p.md:487-492`).

## The full Plan 9 cast, recreated

Once import exists, the rest of the mesh is "fill in the Plan 9 cast, in order" (`docs/mesh-the-missing-half-of-9p.md:451-496`). Each classic Plan 9 service has a Wanix counterpart that rides the same import:

- **factotum → the QUIC handshake.** Plan 9's `factotum` held keys and spoke auth so no program had to. The mesh equivalent is node identity plus capability grants: a node is named by its ed25519 key, and importing its namespace is gated by a capability you were granted, not by where you sit on the network. See [a capability is a bind](/concepts/capability-is-a-bind).
- **venti → `#cas`.** Plan 9's `venti` was a write-once, hash-addressed block store. The Wanix slice is content-addressed blobs: large or shared data referenced by hash, deduplicated and verified, exposed as files like everything else. See [the content-addressed data plane](/concepts/content-addressed-data-plane).
- **cpu(1) → `#cpu`.** Plan 9's `cpu` rebuilt your namespace on a remote machine and ran your shell there with your local devices imported back. The Wanix move is "send the agent to the data": an agent operating `/n/<node>` is already operating a remote namespace by file; `#cpu` relocates the *compute* next to the files. See [send the agent to the data](/concepts/send-agent-to-the-data).
- **plumber → `#plumb`.** Plan 9's `plumber` routed messages between programs by pattern. Across a mesh that becomes best-effort gossip, riding the same identity and transport. See [the plumb device](/devices/plumb).

The transport under all of it is iroh QUIC, which provides the NAT traversal and relays that let `/n/<node>` reach a machine on the open internet instead of just `127.0.0.1`. See [the FileSystem contract over iroh QUIC](/concepts/9p-over-iroh-quic).

## Between two Wanix nodes, import rides the native wire — not 9P

The import half above was first built *as* a 9P client (`RemoteFs`), and that is still how Wanix reaches a *foreign* 9P peer. But between two **Wanix** nodes — both of which speak the `FileSystem` trait natively — tunnelling 9P buys nothing and costs chattiness, single-stream head-of-line blocking, tags, and `msize` negotiation. So the default Wanix↔Wanix mesh path is now the **native FileSystem-over-iroh wire** (the crate `wanix-mesh-wire`, ALPN `wanix/fs/1`): the same `bind` at `/n/<peer>`, but the imported `FileSystem` is a `NativeFs` instead of a `RemoteFs`. It is the import half generalized off 9P.

What the native wire changes, and why it is the right shape for a mesh of file-shaped services:

- **One stream per call, one per open file.** The QUIC stream *is* the transaction — no tags, no `msize`. A never-EOF read (`#agent/<id>/events`, `#plumb/<topic>/recv`) parks only its own stream, so the head-of-line freeze that 9P's serial connection has (and that [`StreamingImportFs`](/concepts/streaming-import-fs) hand-patches) simply cannot happen. Streaming is structural.
- **Typed errors.** `FsError` crosses as a typed `WireFsError`, so `InvalidPath("a/../b")` arrives with its message intact — not flattened to a lossy `errno`, as the 9P round trip does.
- **Identity built in.** The verified ed25519 `remote_id()` binds a per-connection, principal-scoped `FileSystem` view via the capability policy — the principal comes from the transport, never the payload.

9P does **not** go away: it stays the foreign-edge gateway — Linux `v9fs`, v86/QEMU virtio-9p, external 9P tools, the browser cockpit's `p9.ts`, and the `tcp://` mount path. The decision and the op-by-op wire are recorded in [ADR 0004](/reference/adr-index). One trait, two encodings: the native wire inward, 9P at the foreign edge.

## See also

- [RemoteFs, the import half](/concepts/remotefs-import-half) — the one struct that mirrors the server.
- [Import, export, and /n/](/concepts/import-export-and-n) — where a peer's namespace lands in yours.
- [Devices import for free](/concepts/devices-import-for-free) — why no device needed teaching to go remote.
- [Agents as operators](/concepts/agents-as-operators) — why the namespace's natural user is an agent, and what import gives it.
- [Wanix as host](/concepts/wanix-as-host) — the runtime that composes all of this.
- Next: [Plan 9 ideas tour](/learn/plan9-ideas-tour) — walk the lineage end to end.

## Status / honest limits

- **Partial ADR coverage.** The native FileSystem-over-iroh wire is now recorded in [ADR 0004](/reference/adr-index) (FileSystem contract + native mesh wire + 9P edge gateway), with the op-by-op design in `docs/design/native-mesh-wire.md`. The rest of the mesh/agent layer — ed25519 identity, capability binds, and the service device contracts — is still captured in design docs (`docs/mesh-the-missing-half-of-9p.md`), not ratified records. Treat the un-ADR'd parts as current direction, not frozen API.
- **`/n/<peer>` is a convention, not a feature.** The shipped CLI mount binds a *single* slot at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` addressing is designed but not shipped; the multi-peer `/n/A`, `/n/B` paths in this page are a labelled convention for how it will read.
- **Exec across an import is local-trust only.** Reaching `#task`, `#cpu`, or `#agent` over an import gives "cheap, scalable isolation," not "safe for arbitrary untrusted code" — there are no hard CPU or memory limits yet, and these devices are not exposed to untrusted public peers. The served `#agent` is a deterministic `FakeEngine`, not a live LLM; real codex is the local-trust `wanix agent` CLI path only.
- **Tauth stays ENOSYS.** There is no 9P auth handshake; the trust boundary is the capability bind and the QUIC identity, not in-band 9P authentication.
