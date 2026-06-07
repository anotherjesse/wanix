---
title: Agents Are the Operators Namespaces Always Needed
slug: concepts/agents-as-operators
pageType: concept
oneLiner: Per-process namespaces and file-shaped services were too fiddly for humans but native to an LLM that reads by cat, mutates by write, and lists by ls.
audience: [visionary, developer]
tags: [mesh, agent, local-trust-only, exploratory, caveat, why-now]
sourceRefs:
  - docs/mesh-the-missing-half-of-9p.md:98-122
  - docs/mesh-the-missing-half-of-9p.md:2162-2192
  - crates/wanix-agent/src/path.rs:36-45
  - crates/wanix-cli/src/mount.rs:26
seeAlso:
  - devices/agent
  - concepts/approvals-as-files
  - concepts/send-agent-to-the-data
  - concepts/missing-half-of-9p
  - concepts/traceable-namespaces
prerequisites:
  - concepts/per-process-namespaces
  - devices/agent
usedInFlows: []
honestLimits:
  - The served #agent is a deterministic FakeEngine, not a live LLM; real codex runs only on the local-trust 'wanix agent' CLI path.
  - This page is a thesis about why-now; the mesh/agent layer has no ADRs yet, only blueprint docs.
  - Exec devices (#agent/#task/#cpu) are local-trust only, with no hard CPU/memory limits.
  - The shipped CLI mount binds a single /n/remote slot; per-peer /n/<peer> is a labelled convention, not yet shipped.
canonicalCaveatFor: []
---

# Agents Are the Operators Namespaces Always Needed

Per-process namespaces and file-shaped services were too fiddly for humans, but native to an LLM that reads by `cat`, mutates by `write`, and lists by `ls`.

This page is the "why now" behind the whole mesh. Wanix did not add an agent feature on top of a filesystem; it noticed that the two oldest, most underused ideas in Plan 9 — a private namespace per process, and every service shaped as a file — describe exactly the world an LLM operates best in. The tool humans found too sharp to hold at full strength is the tool an agent reaches for naturally. State that claim plainly, show it at the keyboard, and name what is still a thesis rather than a shipped contract.

## The thing humans found too fiddly

Plan 9 gave you a namespace you could rearrange per command and services you composed by bind order. It was *powerful* and it was *underused*. Rearranging your private world before every command, importing a remote device under `/n/`, sequencing binds to stack one service over another — the ergonomic cost of constantly reshaping a private world fell on a person who mostly wanted their shell to stay put (`docs/mesh-the-missing-half-of-9p.md:98-105`). The capability was real; the friction was real; humans paid the friction grudgingly and mostly left the power on the shelf.

An agent does not carry that cost. An LLM operating a confined world as files finds per-process namespaces **native**, not fiddly. It reads state by `cat`, mutates by `write`, lists capabilities by `ls`, spawns work by writing to a control file, and waits on results by reading another. A namespace it can rearrange is just a context it can shape to the task in front of it. The thing humans found too sharp to hold all the time is the thing an agent reaches for naturally (`docs/mesh-the-missing-half-of-9p.md:107-112`). Nothing here is a new abstraction invented for AI — it is the same `FileSystem` contract everything else in Wanix already speaks. (See [everything is a file](/concepts/everything-is-a-file).)

## Wanix already had the agent; what it lacked was reach

The `#agent` device is an LLM you can `cat`. Allocate a session, drive it, watch it — all as files. The path verbs are fixed and small (`crates/wanix-agent/src/path.rs:36-45`):

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

id=$(cat '#agent/new')                          # allocate a session
echo 'fix the failing test' > "#agent/$id/prompt"
cat "#agent/$id/events"                          # streamed turn events
cat "#agent/$id/pending"                         # approvals it is waiting on
cat "#agent/$id/status"
```

That session is not sandboxed off in some API; it operates the *full* services namespace. It edits a confined Wanix world live, reads and writes `#kv`, lists and starts `#task`, attaches `#term`, runs Wanix programs, answers as a network service via `POST /agent`, and even delegates to another agent through `#agent/<id>/reply` (`docs/mesh-the-missing-half-of-9p.md:114-118`). The operator was already here.

What the agent did *not* have was **reach**. It could operate exactly one world — the world it was booted into. The mesh is the part that hands a confined agent the rest of the network. Bind a remote node and the agent's `cat`/`write`/`ls` vocabulary extends, unchanged, to files and services on another machine (`docs/mesh-the-missing-half-of-9p.md:119-122`). Because every device is a plain `FileSystem`, a peer's key/value store is just `/n/<peer>/#kv/<key>` and a peer's agent is `/n/<peer>/#agent/...` — no new client, no new protocol, the import half of 9P doing all the work. (See [the missing half of 9P](/concepts/missing-half-of-9p) and [devices import for free](/concepts/devices-import-for-free).)

One honest note on spelling: the shipped CLI `mount` binds a single slot at `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is the designed shape and the way to *think* about reach, but today it is a labelled convention, not multiple live slots.

## The four verbs are the whole interface

This is the crux. An agent does not learn an SDK; it learns four verbs it already has, applied to files:

- **`cat` to read state** — `cat "#agent/$id/status"`, `cat '#kv/build.flag'`, `cat '#task/self/id'`. Observing the world is reading.
- **`write` to mutate** — `echo 'on' > '#kv/build.flag'`, `echo 'go' > "#agent/$id/prompt"`. Changing the world is writing.
- **`ls` to discover capabilities** — `ls '#task/new'` lists the task kinds it can start; `ls /n/A/#kv` lists a remote peer's keys. Capability discovery is listing.
- **write a control file to spawn work** — write `#task/new` to start a task, write `#agent/.../ctl` to approve a step, post `#plumb/<topic>/send` to nudge a peer. Doing work is writing to the right file.

Approvals fit the same mold: a human approving an agent's risky step is a `write` to a `ctl` file, not a callback in a UI framework (see [approvals as files](/concepts/approvals-as-files)). Coordination fits too — Plan 9's plumber was the broker-less "talk by intent" bus humans wired grudgingly, and an agent finds `cat #plumb/build/recv` then "if `kind == task.done`, go read `body.out`" native, because it is just reading a file and acting on it (`docs/mesh-the-missing-half-of-9p.md:2188-2192`). The plumber is the coordination half Plan 9 always had, and the agent is the operator it always wanted.

## Why this is the strongest "why now"

Plan 9's namespace ideas have been admired and shelved for thirty years on ergonomic grounds. The change is not that the ideas got better; it is that the operator got better. A new kind of process arrived that finds reshaping a private world, composing file-shaped services, and importing remote namespaces *cheaper* than any alternative interface — because for an LLM, "open a file" is the cheap operation and "learn a bespoke API" is the expensive one. Wanix is the bet that the file abstraction, finally, has the operator it was waiting for: send the agent to the data, give it a confined room rather than the whole house, and let every action it takes be a file event you can read back. (See [send the agent to the data](/concepts/send-agent-to-the-data) and [traceable namespaces](/concepts/traceable-namespaces).)

## See also

- [The #agent device](/devices/agent) — the exact files: `new`, `prompt`, `events`, `pending`, `ctl`, `reply`, `status`.
- [Approvals as files](/concepts/approvals-as-files) — why a human-in-the-loop step is a `write`, not a callback.
- [Send the agent to the data](/concepts/send-agent-to-the-data) — `#cpu` and running compute next to a peer's files.
- [The missing half of 9P](/concepts/missing-half-of-9p) — import, and why reach falls out of `RemoteFs`.
- [Traceable namespaces](/concepts/traceable-namespaces) — every agent action is a file event you can audit.
- [Per-process namespaces](/concepts/per-process-namespaces) — the private world the agent reshapes.

## Status / honest limits

- **The served `#agent` is a deterministic `FakeEngine`, not a live LLM.** A real codex engine runs only on the local-trust `wanix agent` CLI path; anything reachable over `serve` answers from a fixed fake (see [FakeEngine vs codex](/concepts/fakeengine-vs-codex)). The demos prove the *file shape* of an agent session, not live model behaviour over the network.
- **This page is a thesis, not a stabilized contract.** The mesh/agent layer has no ADRs yet — its design lives in `docs/mesh-the-missing-half-of-9p.md` and the mesh blueprint. Treat the "why now" as a direction, and the file verbs as the part that is real.
- **Exec devices are local-trust only.** `#agent`, `#task`, and `#cpu` are not exposed to untrusted or public peers; there are no hard CPU or memory limits yet. This is cheap, composable isolation for cooperating nodes, not a claim of safety for arbitrary untrusted code (see [safe-for-untrusted is not yet claimable](/concepts/safe-for-untrusted-not-claimable)).
- **`/n/<peer>` is a convention.** The shipped CLI mount binds one slot, `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer slots are designed but unshipped; use `/n/<peer>` to reason, not to assume.
