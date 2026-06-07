---
title: AI Agents Operating on Your Files Across Devices
slug: use-cases/agents-on-your-files
pageType: use-case
oneLiner: "#agent repairs a broken program with approvals-as-files; over the mesh it reaches a peer's #kv/#cas/files; two agents on two machines collaborate via #plumb."
audience: [visionary, developer]
tags: [agent, mesh, local-trust-only, caveat, shipped, cli]
sourceRefs:
  - docs/recipes/01-repair-broken-qjs.md
  - docs/recipes/05-two-agents-collaborate.md
  - docs/mesh-the-missing-half-of-9p.md:98-122
  - docs/integration/STATUS.md:445-449
  - crates/wanix-cli/src/serve/roots.rs:166
  - crates/wanix-cli/src/agent.rs:83-90
  - crates/wanix-id/src/grant.rs:32-97
  - crates/wanix-cli/src/mount.rs:26
seeAlso:
  - concepts/agents-as-operators
  - devices/agent
  - concepts/approvals-as-files
  - concepts/send-agent-to-the-data
  - concepts/capability-is-a-bind
  - concepts/fakeengine-vs-codex
  - use-cases/personal-compute-mesh
prerequisites:
  - concepts/agents-as-operators
  - devices/agent
  - concepts/approvals-as-files
usedInFlows:
  - {flow: agent-on-your-files, step: 0}
honestLimits:
  - The served #agent is a deterministic FakeEngine, not a live LLM; a real codex engine runs only on the local-trust `wanix agent` CLI path.
  - Exec devices (#agent, #task, #cpu) are local-trust only; never exposed to untrusted or public peers, and there are no hard CPU or memory limits yet.
  - The shipped CLI mount binds one slot at /n/remote; per-peer /n/<peer-id> is designed but unshipped, so /n/<peer> is only a labelled convention.
  - serve handles one 9P frame at a time per connection, so a blocking #plumb recv cannot interleave with a write on the same connection.
canonicalCaveatFor: []
---

# AI Agents Operating on Your Files Across Devices

#agent repairs a broken program with approvals-as-files; over the mesh it reaches a peer's #kv/#cas/files; two agents on two machines collaborate via #plumb.

**What & why.** You want an LLM to actually *do* work on your real files — fix a crashing script, finish a refactor, build against a dataset that lives on another machine — without handing it a blank-check shell. Wanix gives the agent a confined world of files and a single trust gate you can read with your eyes: it proposes an edit, parks for approval, and nothing happens until you write `approve` to a file. Because that world is just a namespace, the same agent reaches across the mesh — bind a peer and the agent's `cat`/`write`/`ls` extend to that peer's files and devices unchanged. The agent is the operator that per-process namespaces and file-shaped services always needed.

## The outcome: an agent that operates your files, safely

Start with the smallest real loop: a qjs program that crashes with a `ReferenceError`, and an agent that fixes it. Serve a workspace with the device set turned on:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

wanix-rust serve \
  --root /tmp/repair-demo \
  --addr 127.0.0.1:7654 \
  --bundle workbench-fs9p \
  --wanix-services
```

`--wanix-services` binds `#agent`, `#task`, `#term`, `#pipe`, `#kv`, `#plumb`, and `#cas` into the served namespace (`crates/wanix-cli/src/serve/roots.rs`). The agent device is bound in that same path — `AgentDevice::new(Arc::new(FakeEngine))` at `crates/wanix-cli/src/serve/roots.rs:166`. Now the entire LLM contract is reads and writes on one tree. Allocate a session by reading `#agent/new`, submit a repair prompt to `#agent/<id>/prompt`, watch `#agent/<id>/events`, and the engine pauses at an `approval.needed` before it touches your file:

```sh
id=$(wanix-rust mount-cat tcp://127.0.0.1:7654 '#agent/new' | tr -d '\n')
wanix-rust mount-write tcp://127.0.0.1:7654 "#agent/$id/prompt" \
  "approve: edit /tmp/repair-demo/agent/broken.js to declare missingValue"
wanix-rust mount-cat   tcp://127.0.0.1:7654 "#agent/$id/pending"
# -> [{"id":"req-1","action":"edit /tmp/repair-demo/agent/broken.js ..."}]
wanix-rust mount-write tcp://127.0.0.1:7654 "#agent/$id/ctl" "approve req-1"
```

That is the whole shape, named after the effect: an LLM session is a directory, a turn is a write, the result is a read, and the trust gate is a write to `ctl`. This is the [`#agent`](/devices/agent) device, and [Recipe 01](/recipes/01-repair-broken-qjs) walks it end to end — the cockpit's "Run Agent Repair Demo" button automates exactly these file operations (`docs/integration/STATUS.md:445-449`).

## Why agents are the operators namespaces always needed

Per-process namespaces and file-shaped services are powerful and historically *underused*: rearranging a private world per command was too fiddly for a human who mostly wanted their shell to stay put (`docs/mesh-the-missing-half-of-9p.md:98-122`). An agent has no such reluctance. It reads state by `cat`, mutates by `write`, lists capabilities by `ls`, spawns work by writing a control file, and waits on results by reading another. A namespace it can rearrange is just a context it can shape to the task. The sharp tool humans held gingerly is the one an agent reaches for naturally — see [agents as operators](/concepts/agents-as-operators).

## Approvals as files: a trust gate you can reason about

Powerful actions never fire silently. When the engine would apply a patch or run a command, it parks the request: it appears in `#agent/<id>/pending` as a JSON array, and nothing executes until a human writes `approve <req>` (or `deny <req>`) to `#agent/<id>/ctl`. The verb parser lives in `CtlFile::write`, which routes through `AgentDevice::resolve` to unblock the turn (Recipe 01, step 4). There is no out-of-band approval channel to audit — the gate *is* a file you read and a file you write. That makes the trust boundary legible: you can `cat pending` from any 9P client and see precisely what the agent is asking permission to do before it does it. This is [approvals as files](/concepts/approvals-as-files).

## Reach: bind a peer and the agent's vocabulary extends

A confined agent can operate the world it booted into. The mesh hands it the rest of the network. Because importing a peer is importing files (every device is a plain `FileSystem`), binding a remote node makes its `#kv`, `#cas`, and ordinary files show up through the same 9P contract — [devices import for free](/concepts/devices-import-for-free). The agent's `cat`/`write`/`ls` now resolve onto another machine, unchanged.

```sh
export NODE_B='iroh://829fbb…f986?addr=127.0.0.1:5680'
wanix-rust mount-ls  "$NODE_B" work
wanix-rust mount-cat "$NODE_B" work/dataset.txt   # bytes that only live on B
```

The aspirational spelling is `/n/<peer>/…`, but the shipped `mount-*` verbs bind one slot — `MOUNT_POINT = "n/remote"` at `crates/wanix-cli/src/mount.rs:26`. The peer identity lives in the `iroh://` ticket you dialed, not in the path prefix; treat `/n/<peer>` as a labelled convention until per-peer roots ship.

Sometimes you want the inverse: move the *agent* to where the data already lives. `#cpu` runs a task on a peer against the caller's reverse-exported namespace — Plan 9 cpu over the mesh, applied to agents. That is [send the agent to the data](/concepts/send-agent-to-the-data) and the broader story in [your personal compute mesh](/use-cases/personal-compute-mesh).

Two agents collaborate the same way: open `#agent/new` twice, write a prompt to each, and have one agent block on `cat #agent/<B>/reply` while B works. `reply` is a single-read, EOF-terminating final message — the contract that makes one agent delegating to another clean (Recipe 05). Across the mesh, the same delegation rides `#plumb` topics between nodes, subject to the single-frame caveat below.

## Capability security: a grant is a re-rooted SubtreeFs

Reach is gated, not ambient. A peer attaches only with a grant, and a grant is not an ACL bolted onto a global tree — it *is* a re-rooting. `Authorization::root` builds `SubtreeFs::new(backing, prefix, rights)` (`crates/wanix-id/src/grant.rs:32-97`), so the granted peer receives a filesystem whose root *is* the subtree. There is no "outside" to name: paths that escape the prefix don't resolve, because from the peer's view they don't exist, and the rights are enforced inside the filesystem itself, not just at the door. A capability is a bind — see [capability is a bind](/concepts/capability-is-a-bind). This is why the trust model is "rooms, not houses": you hand a peer a room, addressed by a verified ed25519 key, with no hallway to wander.

## Runnable recipes

- [Recipe 01 — Repair my broken qjs program with an agent](/recipes/01-repair-broken-qjs): the full approvals-as-files loop, in the cockpit and by hand.
- [Recipe 05 — Two agents collaborate via reply](/recipes/05-two-agents-collaborate): delegation as a blocking `cat` on a peer session's `reply`.

## See also

- [#agent device](/devices/agent) · [agents as operators](/concepts/agents-as-operators) · [approvals as files](/concepts/approvals-as-files)
- [send the agent to the data](/concepts/send-agent-to-the-data) · [capability is a bind](/concepts/capability-is-a-bind) · [FakeEngine vs codex](/concepts/fakeengine-vs-codex)
- [your personal compute mesh](/use-cases/personal-compute-mesh) · [learn: agent on your files](/learn/agent-on-your-files)

## Status / honest limits

- **The served `#agent` is a deterministic `FakeEngine`, not a live LLM.** The cockpit and `serve --wanix-services` path bind `AgentDevice::new(Arc::new(FakeEngine))` (`crates/wanix-cli/src/serve/roots.rs:166`) so the demo runs end to end with no network keys. A real codex `app-server` engine runs only on the local-trust CLI path: `wanix-rust agent …` defaults to `CodexEngine` and falls back to `FakeEngine` under `--fake` (`crates/wanix-cli/src/agent.rs:83-90`). The wire shape is identical; the engine behind it is not.
- **Exec devices are local-trust only.** `#agent`, `#task`, and `#cpu` are remote code execution; they are never exposed to untrusted or public peers. This buys cheap, scalable isolation — not safety for arbitrary untrusted code — and there are no hard CPU or memory limits yet.
- **One mount slot, not per-peer paths.** The shipped `mount-*` verbs bind the remote at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is designed but unshipped; use `/n/<peer>` only as a labelled convention.
- **Live cross-mesh pub/sub needs a second connection.** `serve` handles one 9P frame at a time per connection, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on the same connection. Two agents collaborating live over `#plumb` across the mesh want a second 9P connection or concurrent frame handling.
- **No public multi-user auth.** Attach is default-deny per verified ed25519 key; Tauth stays ENOSYS (there is no 9P auth handshake), and public/multi-user auth remains explicitly unimplemented trust-boundary work.
