---
title: Wire two machines into a mesh
slug: learn/wire-a-mesh
pageType: flow
oneLiner: Mount a peer over iroh QUIC, read its devices for free, send compute to its data, then reason about the trust boundary.
audience: [newcomer]
tags: [mesh, cli, local-trust-only, caveat, shipped]
sourceRefs:
  - docs/recipes/02-mount-remote-peer.md
  - docs/recipes/05-two-agents-collaborate.md
  - docs/recipes/03-freeze-world-to-capsule.md
  - docs/mesh-blueprint.md
  - crates/wanix-cli/src/mount.rs:26
  - crates/wanix-cli/src/mesh/serve.rs:112-137
seeAlso:
  - concepts/remotefs-import-half
  - concepts/9p-over-iroh-quic
  - concepts/key-is-the-address
  - concepts/capability-is-a-bind
  - concepts/send-agent-to-the-data
  - concepts/devices-import-for-free
  - concepts/attach-policy
  - devices/cpu
  - devices/kv
  - devices/agent
prerequisites:
  - learn/js-outside-chrome
usedInFlows: []
honestLimits:
  - The shipped CLI mount binds a single slot /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is designed but unshipped.
  - Exec devices (#task/#agent/#cpu) are local-trust only; not exposed to untrusted public peers, and there are no hard CPU/memory limits yet.
  - The WS-served 9P door handles one frame at a time per connection, so there a blocking cross-node #plumb recv cannot interleave with a write on the same connection; the native mesh wire gives every open file its own stream and does not have this constraint.
canonicalCaveatFor: []
---

# Wire two machines into a mesh

Mount a peer over iroh QUIC, read its devices for free, send compute to its data, then reason about the trust boundary.

This flow takes two machines — call them A and B — and joins them so A can list B's files, read B's `#kv` keys, and run a job *on B against B's local data*, all as ordinary file operations. Nothing here is RPC or a special remote API. A remote namespace mounts as local files; that is the whole trick, and it is the half of 9P Wanix finally ships. You should already be able to run a `qjs` task — finish [JavaScript outside Chrome](/learn/js-outside-chrome) first. Two terminals, about twenty minutes.

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

## Show it: two terminals, one namespace

Give each node a clean key path and a directory to export. Put the code and data on B; leave A empty.

```sh
# Terminal 1 (node B): the data node.
export NODE_B_KEY=/tmp/wanix-B.key
export ROOT_B=$(mktemp -d); mkdir -p "$ROOT_B/work"
printf 'data that only lives on node B\n' > "$ROOT_B/work/dataset.txt"

wanix-rust mesh-serve --root "$ROOT_B" --key "$NODE_B_KEY" \
    --addr 127.0.0.1:5680 --wanix-services
```

B prints its node id and a dialable `iroh://` ticket on stderr (`docs/recipes/02-mount-remote-peer.md`):

```
wanix-rust mesh-serve: mount iroh://829f…<64 hex>…f986?addr=127.0.0.1:5680
```

Copy that line into A. The 64 hex digits after `iroh://` *are* B's ed25519 public key; the `?addr=` carries first-contact direct addresses.

```sh
# Terminal 2 (node A): the importer. A runs no server to import.
export NODE_B='iroh://829f…f986?addr=127.0.0.1:5680'
wanix-rust mount-ls  "$NODE_B" work          # -> dataset.txt
wanix-rust mount-cat "$NODE_B" work/dataset.txt
# -> data that only lives on node B
```

Those bytes never touched A's disk. They crossed a QUIC stream from B and deserialized through the mesh's import half on A. Because this is an `iroh://` mount between two Wanix nodes, the import is a `NativeFs` over the [native FileSystem-over-iroh wire](/concepts/missing-half-of-9p) (one `postcard`-framed stream for the read), not 9P — 9P is the foreign edge, reached over `tcp://`. **Now the name:** this is Plan 9 *import* — A bound B's exported namespace as a local `FileSystem`. The mount lands at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). See [the import half](/concepts/remotefs-import-half).

## Identity: the key is the address

There is no DNS, no username, no IP-as-identity. A node's identity is a 32-byte ed25519 seed at `~/.wanix/node.key` (created `0600`, stable across restarts). The public key is the `PeerId` *and* the iroh `EndpointId`, so the address you dial is the cryptographic identity you verify (`docs/mesh-blueprint.md` §1). The QUIC handshake authenticates B's key before any filesystem frame flows; the server binds that verified key to a per-connection principal-scoped view and never trusts a client-claimed name. That is why there is no in-band auth handshake on either plane (`Tauth` stays ENOSYS on the 9P edge) — there is nothing left to authenticate once the transport already proved the key. More in [the key is the address](/concepts/key-is-the-address).

## Devices import for free

Because every Wanix service device is a plain `FileSystem`, the moment B binds one into its exported namespace, A reaches it through the same mount with no new code. With `--wanix-services`, B exported `#kv` alongside its host root, so A reads a remote key as a file:

```sh
wanix-rust mount-cat "$NODE_B" '#kv/config'    # B's key store, over QUIC
```

This is what [devices import for free](/concepts/devices-import-for-free) means: `#kv`, `#cas`, `#plumb`, and `#agent` all cross nodes the instant they are bound, because the mesh carries the one `FileSystem` contract (the native wire here) and they are all just filesystems. (Keep in mind `#kv` is in-memory — see [the `#kv` device](/devices/kv).)

## Send compute to the data with `#cpu`

Importing files is half of cpu(1); the other half is moving the *computation*. `wanix-rust cpu` dials B's exec plane, reverse-exports A's working directory as a scoped namespace, and asks B to run a task whose world *is* that reverse export. The job runs on B's CPU, against B's local files, with A's files available through the reverse 9P session.

```sh
cd "$ROOT_A"   # empty is fine; the job runs against B's files
wanix-rust cpu --node "$NODE_B" -- qjs /work/build.js
# -> built on node B with B's local files
```

The reverse export is **read-only by default**; pass `--write` to let the remote run write outputs back to A. The exported world is scoped — paths outside the subtree are denied (`docs/recipes/02-mount-remote-peer.md` §4). Two honest edges: output is delivered as a **batch after the task finishes**, not streamed token-by-token, and there is **no remote cancel** — stopping drains the control stream, it does not abort the remote computation. This is [send the agent to the data](/concepts/send-agent-to-the-data); the device page is [`#cpu`](/devices/cpu).

## The trust boundary, flatly

Attach is **default-deny**. A `GrantTable` keyed on the verified `PeerId` decides which subtree each peer may attach; with no grant, attach fails. A capability here is not an ACL entry — *a capability is a bind*. A grant re-roots the host namespace through a `SubtreeFs` at a granted prefix with a rights gate, so a write outside the prefix returns EACCES and a walk can never escape it (`docs/mesh-blueprint.md` §6, [attach policy](/concepts/attach-policy), [a capability is a bind](/concepts/capability-is-a-bind)).

The CLI enforces this at parse time:

- The public endpoint is **refused** without an explicit opt-in: no `--addr`, no `--peer`/`--grant`, no `--insecure-open` is rejected (`crates/wanix-cli/src/mesh/serve.rs:112`).
- `--insecure-open` exports the whole root read-write to the open internet — and grants **no exec**.
- `--wanix-services` is **refused on a public endpoint entirely**, even with `--peer`/`--grant` or `--insecure-open`, because `#task`/`#agent` are remote code execution (`crates/wanix-cli/src/mesh/serve.rs:130-137`).

State the boundary plainly: the exec devices are **local-trust only**. Wanix gives cheap, scalable isolation, not a sandbox safe for arbitrary untrusted code, and there are no hard CPU or memory limits yet.

## Freeze the world to a capsule

Live mesh state is not portable — peers, tickets, and ephemeral fds vanish with the process. To carry a *world* (a directory tree) deterministically, freeze it onto the content-addressed plane:

```sh
export CAS=/tmp/cas-A
ID=$(wanix-rust capsule save "$ROOT_B" --store "$CAS" | awk '/^capsule/ {print $2}')
wanix-rust capsule load "$ID" /tmp/world-B --store "$CAS"
```

Every file becomes a BLAKE3 blob; the sorted manifest's hash *is* the capsule id (`docs/recipes/03-freeze-world-to-capsule.md`). Each blob is re-hashed on load, so a tampered blob is rejected, never served under the wrong address. Note `#kv` is a device, not a directory: to freeze KV state, copy the values you want into the world tree first. See [the `wanix capsule`](/concepts/wanix-capsule) world snapshot.

## Two agents coordinating over the mesh

Agents collaborate the same way everything else does — through files. One agent opens `#agent/new` twice, writes a sub-goal to the second session's `prompt`, then blocks on `cat #agent/$B/reply` for a single final message; `#plumb` carries best-effort handoffs between them (`docs/recipes/05-two-agents-collaborate.md`). Two caveats matter at the mesh edge. First, the served `#agent` runs a deterministic `FakeEngine`, not a live LLM — real codex is the local-trust `wanix agent` CLI path only ([FakeEngine vs codex](/concepts/fakeengine-vs-codex)). Second, the single-frame constraint is plane-specific: over the **native wire** every open file rides its own QUIC stream, so a blocking cross-node `#plumb recv` does *not* block a sibling op; the constraint only bites on the **WS-served 9P** path, where `serve` handles one frame at a time per connection (use a second connection there). Walk it in [two agents collaborate](/recipes/05-two-agents-collaborate).

## The cockpit as a dashboard

The [browser cockpit](/concepts/browser-cockpit) inspects the served devices over direct 9P and is the human operator surface for all of the above. It is a frontend, not a runtime: a remote per-peer `/n/<peer-id>` mesh view inside the cockpit is part of the not-yet-shipped surface, not a current feature.

## See also

- Concepts: [the import half](/concepts/remotefs-import-half) · [9P over iroh QUIC](/concepts/9p-over-iroh-quic) · [the key is the address](/concepts/key-is-the-address) · [a capability is a bind](/concepts/capability-is-a-bind) · [attach policy](/concepts/attach-policy) · [devices import for free](/concepts/devices-import-for-free) · [send the agent to the data](/concepts/send-agent-to-the-data)
- Devices: [`#cpu`](/devices/cpu) · [`#kv`](/devices/kv) · [`#agent`](/devices/agent)
- Recipes: [mount a remote peer](/recipes/02-mount-remote-peer) · [freeze a world to a capsule](/recipes/03-freeze-world-to-capsule) · [two agents collaborate](/recipes/05-two-agents-collaborate)
- Next flow: [the Plan 9 ideas tour](/learn/plan9-ideas-tour)

## Status / honest limits

- **One mount slot, not per-peer.** The shipped `mount-*` verbs always bind the remote at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). The per-peer `/n/<peer-id>` shape is designed but unshipped; treat `/n/<peer>` only as a labelled convention for "the ticket I dialed."
- **Exec is local-trust only.** `#task`/`#agent`/`#cpu` are not exposed to untrusted public peers — `--wanix-services` is refused on a public endpoint (`crates/wanix-cli/src/mesh/serve.rs:130-137`). Isolation is cheap and scalable, not a sandbox for arbitrary untrusted code; there are no hard CPU/memory limits yet.
- **`#cpu` output is batched, with no remote cancel.** Stdout/stderr/exit arrive after the task finishes; stopping drains the control stream rather than aborting the remote run.
- **Single frame per connection (WS-served 9P only).** The WebSocket-served 9P door processes one frame at a time per connection, so there a blocking `#plumb recv` cannot interleave with a write on the same connection — use a second connection for live pub/sub. The **native mesh wire does not have this constraint**: every open file rides its own QUIC stream.
- **The served `#agent` is a deterministic `FakeEngine`,** not a live LLM. Real codex is the local-trust `wanix agent` CLI path only.
