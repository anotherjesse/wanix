---
title: Your Personal Compute Mesh
slug: use-cases/personal-compute-mesh
pageType: use-case
oneLiner: Run a Wanix node on your laptop, phone, and a cloud box, then mount any one of them as local files over iroh QUIC — devices and all — gated by per-peer ed25519 grants.
audience: [visionary, developer]
tags: [mesh, cli, iroh-quic, ed25519, capability, cpu, devices, shipped, local-trust-only, caveat]
sourceRefs:
  - README.md:42-49
  - docs/recipes/02-mount-remote-peer.md
  - docs/mesh-the-missing-half-of-9p.md:1089-1140
  - crates/wanix-cli/src/mount.rs:26
seeAlso:
  - concepts/import-export-and-n
  - concepts/key-is-the-address
  - concepts/capability-is-a-bind
  - concepts/devices-import-for-free
  - concepts/send-agent-to-the-data
prerequisites:
  - concepts/import-export-and-n
  - concepts/key-is-the-address
  - concepts/capability-is-a-bind
usedInFlows:
  - {flow: wire-a-mesh, step: 1}
  - {flow: plan9-ideas-tour, step: 1}
honestLimits:
  - "mount-* verbs always bind the remote at /n/remote, not /n/<peer-id> — there is one mount slot, not per-peer namespace paths yet (MOUNT_POINT in crates/wanix-cli/src/mount.rs:26)."
  - "No mesh panel or browser-as-peer: the cockpit operates only the local node; the verified-peers panel and browser-as-peer mount are designed but unbuilt."
  - "No public multi-user auth — trust is your own keys plus explicit per-peer grants. --wanix-services (which exports #task/#agent remote code execution) is local-trust only and is rejected on the public endpoint; #cpu exec is local-trust-only."
  - "The serve 9P WebSocket handles one frame at a time per connection, so a blocking #plumb/<topic>/recv cannot interleave with a write on the same connection — live cross-mesh pub/sub needs a second connection."
  - "#kv state is in-memory; the served #agent uses a deterministic FakeEngine, not a live LLM (real codex is local-trust CLI only)."
canonicalCaveatFor: [mount-at-n-remote]
---

# Your Personal Compute Mesh

**What & why.** You don't have one computer; you have several — a laptop, a phone, a cloud box — and right now they barely know each other. SSH, rsync, a pile of cloud dashboards, each with its own login. Wanix offers a different shape: every machine runs a Wanix *node*, each node exports its namespace as 9P over a verified QUIC connection, and you mount any peer's files and devices as if they were local. Reading a remote file is a 9P walk-and-read; running a job *where the data lives* is one `#cpu` call. The peer's address is its ed25519 public key — no DNS, no usernames, no IP-as-identity — and nothing crosses the wire until a per-peer grant says it can. This is the Plan 9 dream of "one fabric of files," realized over modern NAT-crossing transport.

## The outcome: one mesh, all yours

Picture three nodes you own. Each runs `wanix mesh-serve`, binds an iroh endpoint from its persistent key, and exports a directory (and optionally its service devices). From any node you can dial any other and *import* its namespace into your own — its files become your files, its `#kv` becomes a key you can read, its CPU becomes a place you can send work.

Because Wanix is "everything is a file" all the way down, importing a peer is importing files. There is no separate "remote API." The dataset on your cloud box, the `#kv` store on your laptop, the `#agent` session on your phone — they all show up through the same 9P contract, mounted into your local tree. That's the whole pitch: **your machines stop being islands and start being one namespace.**

## How it rests on three ideas

This use case is the payoff of three concepts working together. Read them first if any feel unfamiliar:

- **[Import / export and `/n`](/concepts/import-export-and-n).** Plan 9's other half: a server *exports* a namespace, a client *imports* it and binds it into its own tree. `wanix-9p-client`'s `RemoteFs` mounts a remote 9P export as a plain local `FileSystem`; the mesh just carries that same 9P over QUIC instead of loopback TCP.
- **[The key is the address](/concepts/key-is-the-address).** A node's identity is a 32-byte ed25519 seed (`~/.wanix/node.key`, created `0600`, stable across restarts). Its public key *is* its `PeerId` and its iroh `EndpointId`. The dialable ticket is `iroh://<64-hex-pubkey>?addr=IP:PORT` — the hex is the key; the `?addr=` is just a first-contact hint you can drop on the public internet and let iroh's relay discovery resolve from the key alone.
- **[Capability is a bind](/concepts/capability-is-a-bind).** Attach is default-deny. The 9P grant table is keyed on the *verified* `PeerId` (not a client-claimed `uname`), and a grant names a scoped subtree and rights — e.g. peer `2222…` gets `docs` read-only. No grant, no attach: the QUIC connection succeeds, the `Tattach` is rejected.

Those three together mean a mount is a *capability you were granted, addressed by a key you verified, that behaves like a local directory.*

## The two-terminal hero

The fastest way to feel it is two nodes on one laptop. Node B holds the data; node A holds nothing and reaches across. This is [Recipe 02](/recipes/02-mount-remote-peer) end to end — here's the spine.

Node B exports a directory (with a `build.js` and a `dataset.txt` inside `work/`) over a local direct-address endpoint:

```sh
wanix mesh-serve \
    --root "$ROOT_B" --key "$NODE_B_KEY" \
    --addr 127.0.0.1:5680 --wanix-services
```

It prints its verified address on stderr — copy the `ticket` line (yours will differ):

```
wanix mesh-serve: node 829fbb…f986
wanix mesh-serve: ticket iroh://829fbb…f986?addr=127.0.0.1:5680
wanix mesh-serve: mount with: wanix mount-ls 'iroh://829fbb…f986?addr=127.0.0.1:5680'
```

From node A — which doesn't even need to run `mesh-serve` to *import* — mount and read across the wire:

```sh
export NODE_B='iroh://829fbb…f986?addr=127.0.0.1:5680'

wanix mount-ls  "$NODE_B" work          # -> build.js  dataset.txt
wanix mount-cat "$NODE_B" work/dataset.txt
# -> data that only lives on node B
```

Those bytes never existed on A's disk. They crossed the QUIC stream as a 9P `Tread` reply and printed on A. And because devices are filesystems too, `--wanix-services` means `#kv`, `#agent`, and friends ride along the same mount — [devices import for free](/concepts/devices-import-for-free).

### The cockpit as your operator dashboard

The CLI is the hero loop; the browser cockpit (a Code OSS / VS Code web extension under `workbench/`) is the operator surface for a single node's namespace today — it inspects the service devices, runs the `#agent` repair demo, runs a qjs→wasm→qjs duet on one shared filesystem, serves HTTP apps with `#kv`-backed state, and self-checks the device set, all over direct 9P. A dedicated **mesh panel** — verified peers, mount/revoke buttons, mounting *from* the browser as a trusted-loopback peer — is designed in `docs/integration/plan.md` but **not yet shipped.** For now, the mesh is driven from the CLI; the cockpit drives the local node.

## Send the compute to the data

Mounting moves bytes toward you. Sometimes you want the opposite: move the *computation* to where the data already lives. That's `#cpu` — Plan 9's `cpu(1)` over the mesh.

```sh
cd "$ROOT_A"   # empty — that's fine, the job runs against B's files
wanix cpu --node "$NODE_B" -- qjs /work/build.js
# -> built on node B with B's local files
```

`wanix cpu` dials B's exec plane (ALPN `wanix/cpu/1`), **reverse-exports** A's working directory as a scoped namespace, and asks B to run the task with that reverse export as its world. The task runs *on B's CPU, against B's local files*, with A's files reachable through the reverse 9P session. The reverse export is read-only by default; `--write` opts a subtree into read-write so the remote run can write outputs back to A. Paths outside the subtree are denied — the jail invariant is enforced by `ExportScope` in `wanix-cpu`. The grammar is strict: options before `--`, the job command after it, and a missing `--node` is refused at parse time. See [send the agent to the data](/concepts/send-agent-to-the-data) for the agent-shaped version of the same move.

## Status and honest limits

This is real and runnable today, but it is early. Be precise about the edges:

- **One mount slot, not per-peer paths (yet).** The page's aspirational shape is `/n/<peer-id>/…`. The shipped `mount-*` verbs always bind the remote at **`/n/remote`** — see `MOUNT_POINT` in `crates/wanix-cli/src/mount.rs:26`. The peer id lives in the `iroh://` ticket you dialed, not in the namespace prefix. Think of `/n/remote` as "the slot for whichever ticket I'm dialing right now." Per-peer mounts wait on the namespace seam (per-fid root storage, a `NamespaceProvider`) — a queued follow-up in `CLAUDE.md`.
- **No mesh panel / browser-as-peer (yet).** The cockpit operates the local node; the verified-peers panel and browser-as-peer mount are designed (`docs/integration/plan.md`) but unbuilt.
- **No public multi-user auth.** `mesh-serve` refuses to serve the public endpoint without a grant gate, and `--wanix-services` (which exports `#task`/`#agent` — remote code execution) is **local-trust only**: the parser rejects it on the public endpoint. The trust model is your own keys and explicit per-peer grants, not accounts. Public multi-user auth, Ethernet/vnet, and grant lifecycle are explicitly unimplemented trust-boundary work.
- **Live pub/sub needs care.** The serve 9P WebSocket handles one frame at a time per connection, so a blocking `#plumb/<topic>/recv` can't interleave with a write on the same connection — live cross-mesh pub/sub wants a second connection.

None of these undercut the core claim. What ships *today* is: two verified nodes, a NAT-crossing QUIC transport keyed on ed25519 identity, default-deny capability grants, file import that round-trips ls/cat/write (and persists on the server host), devices that import for free, and the `#cpu` exec plane end to end — `mesh-serve --cpu` serves the acceptor behind the exec gate and `wanix cpu` runs a cross-node job against the caller's reverse export.

## Runnable recipe

Walk the whole thing — two nodes, identity, mount, and a `#cpu` job — in [Recipe 02 — Mount a remote Wanix peer and run a job on it](/recipes/02-mount-remote-peer). For the two-process import over QUIC loopback (including a write that materializes on the server host), see `docs/mesh-the-missing-half-of-9p.md:1089-1140`.

## See also / next

- **Concepts:** [Import / export and `/n`](/concepts/import-export-and-n) · [The key is the address](/concepts/key-is-the-address) · [Capability is a bind](/concepts/capability-is-a-bind) · [Devices import for free](/concepts/devices-import-for-free)
- **Learn:** [Wire a mesh](/learn/wire-a-mesh) · [Agent on your files](/learn/agent-on-your-files) · [Plan 9 ideas tour](/learn/plan9-ideas-tour)
- **Next use case:** [Send the agent to the data](/concepts/send-agent-to-the-data) — the `#agent` version of moving compute to where the files are.
