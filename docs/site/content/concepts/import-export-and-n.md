---
title: Import / Export and /n/
slug: concepts/import-export-and-n
pageType: concept
oneLiner: Export served files long ago; RemoteFs is the missing 9P client — bind a peer at /n/<node> and its whole namespace, files and # devices alike, becomes local.
audience: [newcomer, developer, visionary]
tags: [mesh, shipped, caveat, cli]
sourceRefs:
  - docs/mesh-the-missing-half-of-9p.md:63-96
  - crates/wanix-cli/src/mount.rs:25-29
  - crates/wanix-cli/src/mount.rs:149-165
  - crates/wanix-vfs/src/binding.rs:60-82
  - crates/wanix-mesh/src/streaming.rs:1-53
  - docs/recipes/02-mount-remote-peer.md:151-158
  - AGENTS.md:220-226
seeAlso:
  - concepts/remotefs-import-half
  - concepts/devices-import-for-free
  - concepts/9p-over-iroh-quic
  - concepts/streaming-import-fs
  - concepts/key-is-the-address
prerequisites:
  - concepts/remotefs-import-half
usedInFlows:
  - {flow: wire-a-mesh, step: 3}
honestLimits:
  - The shipped mount-* verbs bind one remote at /n/remote (a single slot, crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is designed but unshipped.
  - The peer's identity lives in the iroh:// ticket you dial, not in the namespace prefix; treat /n/<peer> as a labelled convention.
  - A plain RemoteFs multiplexes every op onto one 9P stream; a blocking read (#agent events, #plumb recv) needs StreamingImportFs to avoid freezing the whole import.
canonicalCaveatFor:
  - n-single-slot-mount
---

# Import / Export and /n/

Export served files long ago; `RemoteFs` is the missing 9P client — bind a peer at `/n/<node>` and its whole namespace, files and `#` devices alike, becomes local.

Plan 9's deepest idea is not "everything is a file." It is that the namespace is per-process and you can rearrange it (`docs/mesh-the-missing-half-of-9p.md:63-67`). Two operations build a view: **export** hands a subtree to the network, and **import** takes a remote service and splices it into *your* namespace at a path you choose. Wanix had the export half and the placement half for a long time; the missing piece was a `FileSystem` you could bind that, when resolved, speaks to somebody else. That object is `RemoteFs` for a 9P peer — and, between two Wanix nodes, `NativeFs` over the [native FileSystem-over-iroh wire](/concepts/missing-half-of-9p). Once it exists, a peer's files and devices stop being "the network" and become paths. The worked example below uses the `mount-*` verbs over `tcp://`, so it is the 9P import half; an `iroh://` mount between Wanix nodes is the native wire, identical at the `bind` and namespace layer.

## The two operations, and the path that names them

Show it first. Start a server that exports its namespace:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

wanix serve --listen tcp://127.0.0.1:5640 --root ./serverdir
```

Then, from another process, read a file that only exists on the server:

```sh
wanix mount-cat tcp://127.0.0.1:5640 docs/note.txt
# -> the bytes of ./serverdir/docs/note.txt, fetched over the wire
```

The bytes never existed locally. They crossed the socket as a 9P `Tread`/`Rread` exchange, deserialized through the client in `wanix-9p-client`, and printed here. The CLI doc-comment for these verbs is explicit about what just happened: each one dials a 9P server, builds a `RemoteFs`, binds it into a fresh namespace, and runs one filesystem operation through that namespace (`crates/wanix-cli/src/mount.rs:1-9`).

In Plan 9 the convention for where you place an import is `/n/`. You `import` a machine's files under `/n/<name>` — `/n/sources`, `/n/kremvax` — and from then on a remote file is just a path; `cat /n/lab/dev/mouse` reads a mouse on another machine, with no new API and no per-service client library (`docs/mesh-the-missing-half-of-9p.md:73-79`). Wanix keeps the convention. That is the **import / export and /n/** contract: one transport (9P) plus one placement operation (bind) gives network transparency for free, across every file-shaped service at once.

## bind: how the remote becomes local

The placement step is an ordinary `bind` into a `Namespace`. `RemoteFs` is a `FileSystem`, so binding it is no different from binding `#kv` or a `MemFs`:

```rust
let mut namespace = Namespace::new();
namespace.bind(remote, ".", "n/remote", BindOptions::default())?;
```

That is the exact call the CLI makes (`crates/wanix-cli/src/mount.rs:149-160`). `bind` takes the filesystem, a source subtree (`.` — the remote's whole export), a destination path, and bind options; it validates the source exists by calling `metadata` through the remote, then records the binding (`crates/wanix-vfs/src/binding.rs:60-82`). After that, every namespace operation that resolves into `n/remote/...` turns into a 9P exchange on the wire. Import is realized; `/n/` is back (`docs/mesh-the-missing-half-of-9p.md:90-96`).

## Reading /n/A/#kv/<key> drives a peer's device

Here is where one client becomes load-bearing. Because every Wanix capability is already a plain `FileSystem`, importing a peer imports *all* of them at once — its files *and* its `#`-named devices. A peer's key/value store is `/n/A/#kv/<key>`; its agent is `/n/A/#agent/...`; its task table, terminal, and plumber ride the same import (`AGENTS.md:220-226`). Nobody taught `#kv` how to be remote. You wrote a 9P client once, and every file-shaped service any node exports became reachable (`docs/mesh-the-missing-half-of-9p.md:81-89`). The mesh design names this directly: services as files plus a single bind primitive is what makes "everything is a file" load-bearing rather than cute. See [devices import for free](/concepts/devices-import-for-free).

Pair this with the exec plane and you can also send the computation across: `wanix cpu --node "$NODE_B" -- qjs build.js` runs a task *on B* against the caller's reverse-exported working directory, returning the bytes on the cpu control stream (B serves the plane with `mesh-serve --cpu`). That is Plan 9 cpu(1) over the mesh — see [send the agent to the data](/concepts/send-agent-to-the-data).

## Honest caveat: the shipped mount binds one slot

The recipe's promised path is `/n/<peer-id>/`. The shipped reality is narrower. The `mount-*` verbs always bind the remote at one fixed mount point — `MOUNT_POINT = "n/remote"` (`crates/wanix-cli/src/mount.rs:26`) — in a single mount namespace. There is one slot. The peer's identity is *not* in the namespace prefix; it lives in the `iroh://` ticket you dialed (`docs/recipes/02-mount-remote-peer.md:151-158`). So today, read `/n/remote` as "whichever ticket I dialed," and treat `/n/<peer-id>` or `/n/A` as a *labelled convention* this documentation uses for clarity, not a path the shipped CLI produces.

Per-peer `/n/<peer-id>` mounts are a queued follow-up tied to the 9P session/namespace seam: `handle_attach` currently decodes `uname`/`aname` and discards them, and every fid resolves through one shared server root. A per-principal namespace only lands cleanly alongside per-fid root storage and a first real consumer; it is deliberately *not* added as a no-op seam. Until then, the single slot is the honest shape.

## See also

- [RemoteFs: the import half](/concepts/remotefs-import-half) — the `FileSystem` that speaks 9P when resolved.
- [Devices import for free](/concepts/devices-import-for-free) — why binding a peer imports all its `#` devices at once.
- [9P over iroh QUIC](/concepts/9p-over-iroh-quic) — the transport behind an `iroh://` mount.
- [StreamingImportFs](/concepts/streaming-import-fs) — dedicated streams for blocking imported reads.
- [The key is the address](/concepts/key-is-the-address) — where a peer's identity actually lives.
- Next flow: [Wire a mesh](/learn/wire-a-mesh) — export a node, dial it, and read a peer's files and devices.

## Status / honest limits

- **One slot, not per-peer.** The shipped `mount-*` verbs bind a single remote at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is designed but unshipped; `/n/<peer>` in these docs is a labelled convention only.
- **Identity is in the ticket.** A peer's identity travels in the `iroh://` ticket you dial, not in the namespace prefix (`docs/recipes/02-mount-remote-peer.md:151-158`).
- **Blocking reads: native wire is structural, 9P needs StreamingImportFs.** A plain `RemoteFs` (the 9P import) multiplexes every operation onto one 9P stream, so a near-never-EOF read (`#agent/<id>/events`, `#plumb/<topic>/recv`) would park that stream and freeze the whole import; the 9P-plane fix was `StreamingImportFs`, one dedicated QUIC bidi stream per matched blocking open. The **native wire** makes this structural — every open file already rides its own stream — so the wrapper is retired there and a never-EOF read parks only itself.
- **Exec across the mesh is local-trust.** Importing files is open by capability grant, but the exec plane (`#cpu`, `#task`, `#agent`) is local-trust only; it is cheap, scalable isolation, not a sandbox for arbitrary untrusted code, and there are no hard CPU/memory limits yet.
