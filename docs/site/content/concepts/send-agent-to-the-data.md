---
title: Send the Agent to the Data (Mesh Reach)
slug: concepts/send-agent-to-the-data
pageType: concept
oneLiner: Instead of dragging files to the compute, run the job ON the node holding the data against its fast local namespace, reverse-exporting the caller's files — Plan 9 cpu(1) over the open internet.
audience: [visionary, developer]
tags: [mesh, cpu, agent, local-trust-only, caveat, cli]
sourceRefs:
  - docs/mesh-the-missing-half-of-9p.md:1824-1917
  - docs/mesh-the-missing-half-of-9p.md:2092-2199
  - crates/wanix-cpu/src/scope.rs:69-126
  - crates/wanix-cpu/src/event.rs:44-65
  - crates/wanix-cli/src/cpu/parse.rs:32-110
  - crates/wanix-cli/src/mount.rs:25-29
seeAlso:
  - devices/cpu
  - devices/agent
  - devices/plumb
  - concepts/devices-import-for-free
  - concepts/capability-is-a-bind
  - concepts/import-export-and-n
prerequisites:
  - devices/cpu
  - devices/agent
  - concepts/devices-import-for-free
usedInFlows:
  - {flow: agent-on-your-files, step: 4}
  - {flow: wire-a-mesh, step: 5}
honestLimits:
  - "Exec devices (#cpu, #task, #agent) are local-trust only; the cpu acceptor runs behind a grant allowlist and is not safe to expose to untrusted public peers. There are no hard CPU or memory limits on a remote job yet."
  - "A #cpu cancel stops the caller's control-stream drain; it does NOT abort the running guest on the data node (the task driver has no abort hook)."
  - "The shipped CLI mount binds a single slot /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is designed-but-unshipped. /n/<peer> is a labelled convention here."
  - "The served #agent is a deterministic FakeEngine, not a live LLM; real codex runs only on the local-trust 'wanix agent' CLI path."
canonicalCaveatFor: []
---

# Send the Agent to the Data (Mesh Reach)

Instead of dragging files to the compute, run the job ON the node holding the data against its fast local namespace, reverse-exporting the caller's files — Plan 9 cpu(1) over the open internet.

Every other mesh move pulls data *toward* you: you import a peer's namespace at `/n/<peer>` and your local task reads bytes that crawl back over the wire. That is fine until the tree is large or the link is slow — a single `walk + open + read` is three to five serial round-trips (`docs/mesh-the-missing-half-of-9p.md:1908-1913`). The inverse is the more powerful move: leave the data where it is, and send the *job* to it. The job runs on the node that already holds the source tree, at local-namespace speed, and only the small result comes back. This page is about why that one inversion lets agents and compute compose across machines without inventing anything new.

## Mount there, run here

Start from what you already have. Bind a peer at `/n/<peer>` and a task's everyday verbs reach straight into the peer's devices, because every device is a plain `FileSystem` and the import half is just another `FileSystem` (see [devices import for free](/concepts/devices-import-for-free)):

```sh
# An agent reads a remote key store over the mesh — the verb never changed.
cat '/n/A/#kv/region'           # -> us-east
ls  '/n/A/#cas/have'            # the peer's content store, as files
```

That is *import*: the data moves to you. It works, and for small reads it is the right tool. But when chattiness dominates — a big repo, a slow NAT — you flip it.

## Send the job to the data

The cpu plane runs the inverse. Dial the node that holds the data and hand it a job; the job runs *there*, against that node's fast local namespace, and your own files are reverse-exported back to it over the same connection so the run can read inputs and write outputs through the wire (`docs/mesh-the-missing-half-of-9p.md:1828-1833`):

```sh
cargo build --locked --package wanix-cli
alias wanix='./target/debug/wanix'

# Run a qjs job on the data node; reverse-export the local cwd read-only.
# (The data node serves the exec plane with `mesh-serve --cpu`.)
wanix cpu --node iroh://A -- qjs build.js
```

The grammar is `cpu --node TICKET [--cwd DIR] [--write] [--env KEY=VALUE ...] -- KIND PROGRAM [ARG ...]` (`crates/wanix-cli/src/cpu/parse.rs:32-39`): options precede the `--` separator, and everything after it is the job command, so the program's own flags are never confused for cpu options. The captured stdout, stderr, and exit code come back on a separate control stream while the namespace traffic rides its own.

After the effect is visible, here is the name: this is Plan 9's **cpu(1)**, generalized to the open internet. cpu(1) logged you into a remote CPU server, started a shell there, and reverse-mounted your terminal's namespace back onto the remote machine — so the remote shell saw *your* files. Wanix does the same with QUIC instead of a local link and a scoped, content-jailed subtree instead of your whole terminal (`docs/mesh-the-missing-half-of-9p.md:1884-1903`). The compute follows the data; the namespace follows the caller.

## The default jail: read-only, opt into --write

The reverse export is not your whole host root. The caller builds an `ExportScope` over a single subtree and gates it **read-only by default** (`crates/wanix-cpu/src/scope.rs:69-82`). A file outside that subtree is not denied with an error — it is simply *not present* in the exported namespace:

```rust
// Read-only is the default per the cpu correction.
pub fn new(backing: Arc<dyn FileSystem>, prefix: impl Into<String>) -> Self { … }
// Opt the subtree into read-write — the non-default case.
pub fn writable(mut self) -> Self { … }
// Mount an explicitly granted service (e.g. #kv) at its # path.
pub fn grant(mut self, service: GrantedService) -> Self { … }
```

`writable()` opts the subtree into read-write so a build's outputs can land back; `grant(...)` mounts an explicitly named service into the job's world at its `#`-path (`crates/wanix-cpu/src/scope.rs:84-99`). At the keyboard, that is the `--write` flag (`crates/wanix-cli/src/cpu/parse.rs:86-91`):

```sh
# Let the remote build write its outputs back into your subtree.
wanix cpu --node iroh://A --cwd work --write -- wasm build.wasm
```

This is a [capability as a bind](/concepts/capability-is-a-bind), not an ACL check: the scope materializes as a `SubtreeFs` re-rooted at the prefix and rights-gated, then bound into a fresh `Namespace` (`crates/wanix-cpu/src/scope.rs:111-125`). What you did not name is unreachable because it was never bound, not because a guard said no.

## Two agents, two machines, one conversation

The same inversion makes the `#agent` device remote. "Run the agent here" and "run it on node A" are the same call with a different route (`docs/mesh-the-missing-half-of-9p.md:2133-2140`): a `RemoteEngine` drives an imported `#agent` as files — write `<id>/prompt`, read `<id>/reply` — over a plain `FileSystem` handle. So an agent on your laptop can delegate a turn to an agent on the data node, which works against *its* local files, and reply over the mesh.

To make two agents *hand work off* without either polling the other, the mesh adds the **plumber** ([`#plumb`](/devices/plumb)). One agent posts a typed message — `#plumb/build/send` with `{kind: "task.done", body: {out: "…"}}` — and an agent subscribed to that topic on another machine picks it up (`docs/mesh-the-missing-half-of-9p.md:2104-2115`). It is best-effort, broker-less coordination by message *type*; `#plumb` is the nudge ("the artifact is ready"), and the durable artifact rides `#kv` or a content-addressed capsule blob, not the bus (`docs/mesh-the-missing-half-of-9p.md:2181-2186`).

## The composition: #agent + #cpu + mesh

Nothing above is a new mechanism. The job's world is *just a `Namespace`*; its world root is *just a bound `FileSystem`*; the export is *just a `P9Server`* running in the reverse direction (`docs/mesh-the-missing-half-of-9p.md:1915-1917`). The mesh added a *direction* — the 9P server now runs on the caller — not a new abstraction. Stack the three primitives and you get a property no single device has:

- **`#cpu`** sends a task to the data node and reverse-exports the caller's subtree.
- **`#agent`** is a task whose body is an LLM session driven as files.
- **the mesh** carries both across identity-bound QUIC, and imports every device for free.

Compose them and an agent runs *on the machine that holds the repo*, edits that repo at local speed, hands the result to a second agent on a third machine through `#plumb`, and the durable output lands in `#kv`. Agents and compute compose across machines through the one 9P contract, because each piece was a file the whole time.

## See also

- [#cpu device](/devices/cpu) — the exec plane, acceptor, and `ExportScope` reference.
- [#agent device](/devices/agent) — the LLM-session-as-files device the mesh routes.
- [#plumb device](/devices/plumb) — the typed message bus two agents hand off through.
- [Devices import for free](/concepts/devices-import-for-free) — why binding `/n/<peer>` makes every device remote.
- [A capability is a bind](/concepts/capability-is-a-bind) — the `SubtreeFs` jail as a grant, not an ACL.
- [Import, export, and /n](/concepts/import-export-and-n) — the import half this page inverts.
- Flow: [Agent on your files](/learn/agent-on-your-files) · [Wire a mesh](/learn/wire-a-mesh).

## Status / honest limits

State these flatly; they are engineering boundaries, not apologies.

- **Exec devices are local-trust only.** `#cpu`, `#task`, and `#agent` grant filesystem and execution capability, so the cpu acceptor runs behind a grant allowlist and is *not* exposed to untrusted public peers. The model is "cheap, scalable isolation," not "safe to run arbitrary untrusted code." There are no hard CPU or memory limits on a remote job yet.
- **Cancel does not abort the remote guest.** A `CpuEvent::Cancel` stops the caller's control-stream drain; the task driver has no abort hook, so the guest on the data node still runs to completion (`crates/wanix-cpu/src/event.rs:44-65`). This is documented, not faked.
- **The served `#agent` is a deterministic `FakeEngine`, not a live LLM.** Real codex runs only on the local-trust `wanix agent` CLI path; the served and mesh-imported `#agent` is deterministic.
- **The shipped CLI mount binds one slot, `/n/remote`** (`crates/wanix-cli/src/mount.rs:25-29`). Per-peer `/n/<peer-id>` is designed but unshipped; `/n/<peer>` in the examples above is a labelled convention, not a literal shipped path.
