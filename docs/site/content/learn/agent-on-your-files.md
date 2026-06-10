---
title: The agent on your files
slug: learn/agent-on-your-files
pageType: flow
oneLiner: Drive an LLM session as files, gate it with file-based approvals, and reach it across the mesh.
audience: [visionary, developer]
tags: [agent, mesh, cli, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-agent/src/lib.rs:87-172
  - docs/recipes/01-repair-broken-qjs.md
  - docs/recipes/05-two-agents-collaborate.md
  - crates/wanix-cli/src/mount.rs:26
seeAlso:
  - devices/agent
  - concepts/agents-as-operators
  - concepts/approvals-as-files
  - concepts/send-agent-to-the-data
  - concepts/fakeengine-vs-codex
  - concepts/rooms-not-houses
  - recipes/01-repair-broken-qjs
  - recipes/05-two-agents-collaborate
prerequisites:
  - learn/js-outside-chrome
usedInFlows: []
honestLimits:
  - The served #agent is a deterministic FakeEngine; the real codex engine is the local-trust 'wanix agent' CLI path only.
  - Exec devices (#agent/#task/#cpu) are local-trust only — there are no hard CPU or memory limits yet.
  - The shipped CLI mount binds a single slot /n/remote; per-peer /n/<peer-id> is designed but unshipped.
canonicalCaveatFor: []
---

# The agent on your files

Drive an LLM session as files, gate it with file-based approvals, and reach it across the mesh.

An agent in Wanix is not a chat box bolted onto an editor. It is an operator that reads and writes the same files you do, parked behind a `ctl` gate you control. This flow takes you from a one-session repair to two agents collaborating across the mesh — and is flat about where the served engine stops being a real model. Build once, then follow the route:

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix-rust='./target/debug/wanix-rust'
```

## Agents are operators, not chat boxes

The thesis: if everything is a file and every process composes its own namespace, then an LLM session is just another set of files in that namespace. You do not call an SDK — you `cat` and `echo`. The whole contract is the `#agent` tree: reading `new` allocates a session and returns its id, then `<id>/prompt`, `<id>/events`, `<id>/pending`, `<id>/ctl`, `<id>/reply`, and `<id>/status` drive it (`crates/wanix-agent/src/lib.rs:200-227`). Because the device is a plain `FileSystem`, the same reads and writes work whether the session lives in this process or three nodes away. See [agents as operators](/concepts/agents-as-operators).

## #agent as files; approvals as files

Start a serve with the cockpit bundle, the service devices bound in, and a loopback raw-9P door (`--p9`) so the host-side `mount-*` verbs below have something to dial — `#agent/...` paths live *inside the served namespace*, not in your host shell:

```sh
mkdir -p /tmp/repair-demo
wanix-rust serve --root /tmp/repair-demo --listen 127.0.0.1:7654 \
  --p9 127.0.0.1:7664 --bundle workbench-fs9p --wanix-services
```

Now drive a repair by hand against the `#agent` files, through the 9P door. Allocate a session, submit a prompt, read the parked approval, then resolve it:

```sh
B=tcp://127.0.0.1:7664
A=$(wanix-rust mount-cat $B '#agent/new' | tr -d '\n')
wanix-rust mount-write $B "#agent/$A/prompt" \
  "approve: edit broken.js to declare missingValue"
wanix-rust mount-cat   $B "#agent/$A/pending"
# [{"action":"edit broken.js to declare missingValue","id":"req-1"}]
wanix-rust mount-write $B "#agent/$A/ctl" "approve req-1"
wanix-rust mount-cat   $B "#agent/$A/status"     # fake idle turns=1
```

(`<id>/events` is the live JSONL stream of the same turn — but it is a *stream*, and a collected `mount-cat` of it blocks until the session closes; the `pending` snapshot is the non-blocking way to see the parked request. Inside a Wanix shell, plain `cat`/`echo` on these paths work directly.)

Nothing touches your file until you write `approve req-1`. That is the whole trust handoff, and it lives in code, not in a prompt: `AgentDevice::resolve` (`crates/wanix-agent/src/lib.rs:116-118`) is the only path from a pending request to an executed action. The Plan 9 term is the same idea you already know — a control file (`ctl`) accepting verbs (`approve`, `deny`, `close`). See [approvals as files](/concepts/approvals-as-files).

The cockpit's "Run Agent Repair Demo" is the one-click proof of this gate: it pops a modal at `approval.needed`, waits for your click, then refreshes the file tree with a before/after diff. The button is a ~70-line client over the exact files above — the device is the contract, the cockpit is one client. Full walkthrough: [recipe 01](/recipes/01-repair-broken-qjs).

## Reach the agent across the mesh

Mesh setup is its own flow — do [wire a mesh](/learn/wire-a-mesh) first; and note up front that the shipped CLI mounts a peer at the single slot `/n/remote`, so read any `/n/<peer>` spelling below as a labelled convention, not a path you can type today. Because `#agent` is a filesystem and the mesh carries the one `FileSystem` contract over QUIC, an agent on a peer is reachable as files from here. The pattern is "run there, namespace from here": [`#cpu`](/devices/cpu) runs a task against your reverse-exported namespace, so the compute sits next to the data while the agent still edits *your* files. [`#plumb`](/devices/plumb) carries the coordination envelopes between sessions. See [send the agent to the data](/concepts/send-agent-to-the-data).

[Recipe 05](/recipes/05-two-agents-collaborate) shows two agents on one refactor: agent A renames `foo()` to `bar()`, wants a second opinion, opens `#agent/new` a second time, hands agent B a narrow sub-goal, then blocks on B's `reply` (namespace paths again — agent A runs *inside* the node, so for it these are plain file reads and writes):

```sh
B=$(cat '#agent/new')
cat > "#agent/$B/prompt" <<'EOF'
Grep for remaining `foo` call sites and run cargo test; report MISSED/TESTS/NOTES.
EOF
findings=$(cat "#agent/$B/reply")   # blocks until B's turn completes, then EOF
```

`reply` is a single-read, EOF-terminating final message — that one property is what makes delegation clean (`crates/wanix-agent/src/lib.rs:161-164`). To A, B's answer is just bytes off a file, no different from reading a test log.

Note the mesh address convention. The shipped CLI `mount` binds remotes into a single slot, `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is designed but not yet shipped, so read `/n/<peer>` as a labelled convention, not a path you can type today. See [import, export, and /n](/concepts/import-export-and-n).

## Capability security you can reason about

A capability here is a bind, not an ACL. When you grant a peer access, you re-root its view onto a `SubtreeFs` confined to a prefix — the agent literally cannot name a path above its room. Attach is default-deny: nothing is reachable until a grant binds it. This is the property that makes "an agent operating on your files" tractable to reason about — you can see the exact subtree it was handed. See [capability is a bind](/concepts/capability-is-a-bind).

## Rooms, not houses — the cost story

The isolation story is "rooms, not houses": each agent gets a confined namespace cheaply, so spinning up a second session (recipe 05's agent B) is as light as reading `#agent/new` again. That buys cheap, scalable composition of many small operators. It is *not* a sandbox for arbitrary untrusted code. The exec devices — `#agent`, `#task`, `#cpu` — are local-trust only and are never exposed to untrusted or public peers, and there are no hard CPU or memory limits yet. See [rooms, not houses](/concepts/rooms-not-houses).

## Provenance roadmap (exploratory)

The longer arc, clearly speculative: because every action an agent takes is a read or write on a named file, a namespace can in principle be made *traceable* — every edit attributable to the capability that authorized it. Today that is an idea the file-shaped design enables, not a feature you can turn on.

## Land it

Pick the runnable artifact that matches your goal: drive [recipe 01](/recipes/01-repair-broken-qjs) for the single-agent repair with the cockpit's one-click gate, or [recipe 05](/recipes/05-two-agents-collaborate) for two agents collaborating over the same filesystem. Both reduce the entire LLM contract to read and write on `#agent`.

## See also

- Devices: [`#agent`](/devices/agent) · [`#cpu`](/devices/cpu) · [`#plumb`](/devices/plumb)
- Concepts: [agents as operators](/concepts/agents-as-operators) · [approvals as files](/concepts/approvals-as-files) · [send the agent to the data](/concepts/send-agent-to-the-data) · [FakeEngine vs codex](/concepts/fakeengine-vs-codex) · [rooms, not houses](/concepts/rooms-not-houses) · [capability is a bind](/concepts/capability-is-a-bind)
- Recipes: [repair a broken qjs program](/recipes/01-repair-broken-qjs) · [two agents collaborate](/recipes/05-two-agents-collaborate)
- Prerequisite flow: [JavaScript outside Chrome](/learn/js-outside-chrome)

## Status / honest limits

- The served `#agent` (under `--wanix-services`) uses a deterministic `FakeEngine`, not a live LLM. The real codex engine runs only on the local-trust `wanix agent` CLI path; the wire shape is identical, so everything you learn here transfers (`crates/wanix-agent/src/lib.rs:87-94`, [FakeEngine vs codex](/concepts/fakeengine-vs-codex)).
- `#agent`, `#task`, and `#cpu` are local-trust only — never exposed to untrusted or public peers. The isolation is cheap and composable, not a sandbox for arbitrary untrusted code, and there are no hard CPU or memory limits yet.
- The shipped CLI mount binds a single slot, `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is designed but unshipped; use `/n/<peer>` only as a labelled convention.
- The traceable-namespace provenance story is exploratory: enabled by the file-shaped design, not a shipped feature.
