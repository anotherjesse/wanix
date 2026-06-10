---
title: Traceable, Forkable, Rewindable Namespaces (Exploratory)
slug: use-cases/traceable-namespaces
pageType: use-case
oneLiner: An agentic runtime should make the chain prompt->plan->command->task->log->mutation->result visible, addressable, and forkable, so you can ask "what changed, and why?" and rewind or fork the world.
audience: [visionary]
tags: [traceability, mesh, exploratory, caveat]
sourceRefs:
  - docs/traceable-dynamic-namespaces.md:1-13
  - docs/traceable-dynamic-namespaces.md:15-117
  - docs/traceable-dynamic-namespaces.md:517-527
  - crates/wanix-plumb/src/lib.rs:1-16
  - crates/wanix-plumb/src/envelope.rs:1-40
  - crates/wanix-plumb/src/files.rs:1-67
seeAlso:
  - concepts/traceable-namespaces
  - concepts/agents-as-operators
  - concepts/approvals-as-files
  - concepts/wanix-capsule
  - devices/plumb
prerequisites:
  - concepts/traceable-namespaces
  - concepts/agents-as-operators
usedInFlows: []
honestLimits:
  - "This page describes an EXPLORATORY product vision; the backing document is explicitly NOT an ADR."
  - "Only a first slice exists today: the #plumb device and the qjs-shell mutation frame. The volume/snapshot/fork verbs in the examples are illustrative, not shipped commands."
  - "#plumb delivery is best-effort epidemic gossip, not a durable queue: a subscriber not listening when a message was sent never sees it, and there is no acknowledgement."
  - "serve handles one 9P frame at a time per connection, so a blocking #plumb recv cannot interleave with a write on the same connection."
---

# Traceable, Forkable, Rewindable Namespaces (Exploratory)

An agentic runtime should make the chain prompt->plan->command->task->log->mutation->result visible, addressable, and forkable, so you can ask "what changed, and why?" and rewind or fork the world.

**What & why.** Trust in an agent is not only about permission prompts. It is about *legibility*: being able to see the path from intent to effect, and to step backward when the effect was wrong. As agents write files, run commands, start services, and chain those actions together, the question that decides whether the system is magical or unnerving is a plain one — *why is the world like this?* Wanix already treats the important system pieces as files: tasks in `#task`, terminals in `#term`, served roots, qjs and wasm runtimes, 9P mounts. This use case extends that shape into a causal model: every mutation points back to the command, task, or agent turn that caused it, and managed state can be mounted as it was before. This is the conceptual substrate behind the cockpit's Activity feed — and it is deliberately labeled exploratory (`docs/traceable-dynamic-namespaces.md:13`), not a committed contract.

## The outcome: legibility, not just permission prompts

Picture an agent that just ran ten commands against your repository. It edited source, launched a server, queried a store, and reported success. Today you reconstruct what happened from terminal scrollback, file diffs, and hidden tool calls — a weak foundation for real work (`docs/traceable-dynamic-namespaces.md:79-83`). The vision is that you instead ask the system directly:

```text
Show me what the agent changed.
Open the system as it was before that command.
Fork the exact state from this failing test.
Compare this route before and after the repair.
Restore the workspace to the moment before task 42.
```

The north star is two sentences: **everything Wanix owns can rewind, and everything Wanix touches can be traced** (`docs/traceable-dynamic-namespaces.md:44-45`). The unit of work stops being "the repo" or "the container" and becomes the *namespace* — the live shape of which volumes are mounted, which versions are visible, which tasks ran against them, which logs and requests were part of the work, and which mutations resulted.

## The causal chain, made visible and addressable

The bet is that the path from prompt to effect should be a first-class, addressable object (`docs/traceable-dynamic-namespaces.md:107-115`):

```text
prompt -> plan -> command -> task -> terminal log -> filesystem mutation
       -> HTTP route change -> test result -> final state
```

If every link in that chain is a file with a stable address, then provenance, reviewability, reversibility, and reproducibility all fall out of the existing "everything is a file" model rather than a bespoke UI bolted on top. Each user, agent, or task could work in its own namespace fork and merge intentionally; a destructive restore is avoidable by default because the past is just *another namespace you can mount* (`docs/traceable-dynamic-namespaces.md:90-104`). The intended user-facing surface stays small — illustrative, not shipped:

```sh
volume create git app --repo ./app.git --branch main --commit task
mount app /app
task run qjs /app/fix.js
ns fork trace:task/42:before --name before-fix
mount ns:before-fix /time/before
```

These verbs are a sketch of where the model points, not commands the binary accepts today. Read them as the shape the substrate is reaching for.

## Managed state can rewind; external effects can be traced

The honest dividing line is this: Wanix can *rewind* the state it owns, and *trace* the effects it does not. Managed state — a `memfs`, a content-addressed snapshot, a git-backed volume — can be forked, mounted side by side, and restored, because Wanix holds the bytes. External effects — an HTTP request that left the machine, a row written to a database it does not own — cannot be un-sent, but they can be *recorded* and correlated with the command that produced them. The first proof point is git-backed volumes, but the deeper idea is broader: a small operating system that remembers why it changed and lets you keep working from any meaningful moment (`docs/traceable-dynamic-namespaces.md:517-526`). [QuickJS snapshots are VM images](/concepts/quickjs-snapshots-are-vm-images) and the [Wanix capsule](/concepts/wanix-capsule) are the concrete "freeze a world" primitives this vision composes from.

## The first shipped slice: #plumb provenance and the mutation frame

This is not purely speculative anymore. A concrete first slice ships: the [`#plumb`](/devices/plumb) device and the [qjs-shell](/concepts/qjs-shell) mutation frame implement a traceable plumber boundary, so shell-driven mutations and external effects flow through one observable wire and namespace changes can be correlated with the commands that caused them (`docs/traceable-dynamic-namespaces.md:1-9`).

The mechanism is Plan 9's plumber. Each `#plumb/<topic>/send` write publishes one newline-JSON envelope — `{kind, from, to, body}` — to a named topic, and each `#plumb/<topic>/recv` read drains the envelopes that topic received since the file was opened (`crates/wanix-plumb/src/lib.rs:3-9`). The `kind` field is the typed routing tag (e.g. `task.done`), `from` and `to` are optional addresses, and `body` is the free-form payload (`crates/wanix-plumb/src/envelope.rs:5-7`). Unknown JSON fields are rejected, so a typo surfaces as an error instead of a silent drop. `send` is strictly write-only and `recv` strictly read-only and blocking — the two ends are deliberately unidirectional (`crates/wanix-plumb/src/files.rs:1-6`). That is the wire that carries real provenance through the system today; the cockpit Activity feed is its user-facing surface (`docs/traceable-dynamic-namespaces.md:6-9`).

Because `#plumb` is a plain `FileSystem`, it imports across the mesh for free: `/n/<peer>/#plumb/<topic>/recv` reads another node's bus as ordinary files ([devices import for free](/concepts/devices-import-for-free)). Treat `/n/<peer>` as a labelled convention — the shipped CLI binds a single `/n/remote` slot, not per-peer paths.

## See also

- Concepts: [traceable namespaces](/concepts/traceable-namespaces) · [agents as operators](/concepts/agents-as-operators) · [approvals as files](/concepts/approvals-as-files) · [best-effort epidemic delivery](/concepts/best-effort-epidemic-delivery) · [the Wanix capsule](/concepts/wanix-capsule)
- Devices: [`#plumb`](/devices/plumb) · [`#agent`](/devices/agent) · [`#task`](/devices/task)
- Use cases: [agents on your files](/use-cases/agents-on-your-files) · [portable worlds](/use-cases/portable-worlds)
- Learn: [agent on your files](/learn/agent-on-your-files) · [the Plan 9 ideas tour](/learn/plan9-ideas-tour)

## Status / honest limits

- **This is an exploratory product vision, clearly labeled.** The backing document is "an exploratory product and architecture note, not an ADR" (`docs/traceable-dynamic-namespaces.md:13`). The volume/snapshot/fork verbs above are illustrative sketches, not commands the binary accepts. Do not read them as a shipped feature list.
- **Only a first slice exists.** What ships today is the `#plumb` device (`crates/wanix-plumb/src/lib.rs`) and the qjs-shell mutation frame — a traceable plumber boundary, not the full rewind/fork model. The general causal store, named-volume snapshots, and `ns fork` are open questions (`docs/traceable-dynamic-namespaces.md:502-515`), not deliverables.
- **`#plumb` is best-effort gossip, not a durable queue.** A subscriber that was not listening when a message was broadcast never sees it, and there is no acknowledgement (`crates/wanix-plumb/src/envelope.rs:7-9`). Durable handoff belongs in `#kv` or a content-addressed capsule blob — and `#kv` itself is in-memory, lasting only as long as the serve process unless you freeze a capsule.
- **Live receive needs a second connection.** `serve` handles one 9P frame at a time per connection, so a blocking `#plumb/<topic>/recv` cannot interleave with a write on the same connection — see the [single-frame serve caveat](/concepts/single-frame-serve-caveat).
- **Exec devices are local-trust only.** When agents and tasks drive these traces, remember that `#task`, `#agent`, and `#cpu` are not exposed to untrusted peers; cheap isolation is not a claim of safety for arbitrary untrusted code, and there are no hard CPU/memory limits yet.
