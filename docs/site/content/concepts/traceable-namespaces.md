---
title: "Traceable / Forkable Namespaces (Provenance, Rewind)"
slug: concepts/traceable-namespaces
pageType: concept
oneLiner: "Make agent work legible: every mutation points back to the command, task, or agent turn that caused it, and the past becomes a place you can mount, diff, fork, or restore."
audience: [visionary, developer]
tags: [mesh, agent, exploratory, caveat, provenance]
sourceRefs:
  - docs/traceable-dynamic-namespaces.md:15-117
  - docs/traceable-dynamic-namespaces.md:189-198
  - docs/traceable-dynamic-namespaces.md:272-275
  - docs/traceable-dynamic-namespaces.md:1-9
seeAlso:
  - concepts/agents-as-operators
  - concepts/approvals-as-files
  - concepts/wanix-capsule
  - devices/plumb
  - use-cases/traceable-namespaces
  - concepts/browser-cockpit
prerequisites:
  - concepts/agents-as-operators
  - devices/plumb
usedInFlows: []
honestLimits:
  - "This is an exploratory product and architecture vision, not a shipped feature; the source note states plainly it is 'not an ADR'."
  - "The volume/trace/rewind object model (#volume, #ns, #trace, ns fork, ns restore) is a design sketch, not implemented surface."
  - "Only a thin first slice exists: #plumb provenance plumbing and the qjs-shell mutation frame correlate some shell-driven mutations with the commands that caused them."
  - "Managed Wanix state can in principle rewind; external effects (email, network, host writes) can only be traced, never un-happened."
canonicalCaveatFor: [traceable-namespaces-exploratory]
---

# Traceable / Forkable Namespaces (Provenance, Rewind)

Make agent work legible: every mutation points back to the command, task, or agent turn that caused it, and the past becomes a place you can mount, diff, fork, or restore.

When an agent can write files, run commands, start services, and chain those actions faster than you can read scrollback, the runtime beneath it should become *more* inspectable, not less. The idea on this page is to make a namespace not just a path lookup table but the live, recorded shape of a working world — so you can ask "why is the system like this?" and get an answer that points all the way back to intent. This is a north-star vision for Wanix, not a shipped device. Read it as direction, and note exactly where the shipped code already touches the idea.

## The chain you want to be able to trace

Today an agent run is held together by terminal scrollback, file diffs, hidden tool calls, and ad hoc summaries (`docs/traceable-dynamic-namespaces.md:79-83`). That is fine for a demo and a weak foundation for real work. The thing that turns a magical system into a trustworthy one is a visible, addressable causal chain (`docs/traceable-dynamic-namespaces.md:108-116`):

```text
prompt -> plan -> command -> task -> terminal log -> filesystem mutation
       -> HTTP route change -> test result -> final state
```

If every link in that chain is a thing you can point at — and every file mutation carries a back-pointer to the task and agent turn that produced it — then agent work becomes something you can debug, audit, undo, and build on. Trust here is not only about permission prompts at the front door. Trust is also legibility: being able to see the path from intent to effect after the fact.

## Show me what the agent changed; restore to before task 42

Wanix already treats the important system pieces as files: tasks in `#task`, terminals in `#term`, served roots, qjs and wasm runtimes, 9P mounts (`docs/traceable-dynamic-namespaces.md:85-88`). Traceable namespaces *extend* that shape rather than replacing it. Once a namespace is the unit of work — "an actor operating in a namespace over time" rather than a process or a Git commit — a small set of operations become natural to ask for (`docs/traceable-dynamic-namespaces.md:34-45`):

```text
Show me what the agent changed.
Open the system as it was before that command.
Fork the exact state from this failing test.
Compare this route before and after the repair.
Restore the workspace to the moment before task 42.
```

The design sketch expresses these as ordinary file plumbing, before any polished UI exists. A `#trace` device would expose the causal graph as files, a `#ns` device would expose the mount table and snapshots, and a `#volume` device would expose named backing stores (`docs/traceable-dynamic-namespaces.md:329-358`). The most *magical* default is deliberately not destructive time travel — it is live forking (`docs/traceable-dynamic-namespaces.md:189-197`):

```sh
ns fork trace:task/42:before --name before-repair
mount ns:before-repair /time/before-repair
```

Now the past is a *place*. You can browse it, run against it, diff it, serve it, or hand it to another agent without destroying the present. Restorative rewind — `ns restore session trace:task/42:before` — stays explicit precisely because it changes what the live session sees (`docs/traceable-dynamic-namespaces.md:256-262`).

To be unambiguous: none of the `ns`/`mount`/`#trace`/`#volume` commands above are shipped. They are the design's proposed shape, reproduced so the vision is concrete.

## The honest rule: managed state rewinds, external effects only trace

The vision does not pretend everything is reversible. Different backing stores support different depths of rewind: a Git volume can restore by commit, an Automerge volume by heads, a database volume by transaction checkpoint, a `memfs` volume only while its snapshot is retained, and a `hostfs` mount is traceable but not rewindable unless Wanix keeps an explicit shadow log (`docs/traceable-dynamic-namespaces.md:264-268`). That collapses to one rule (`docs/traceable-dynamic-namespaces.md:272-275`):

```text
Managed Wanix state can rewind.
External effects can be traced.
```

Sending an email, posting to a service, charging a card, or mutating an unmanaged host directory cannot be made un-happened by wishful naming (`docs/traceable-dynamic-namespaces.md:277-280`). Wanix can still *record* that the effect happened, with the responsible trace context and request/response metadata where it is safe to keep — but recording an effect and undoing it are different promises, and the model refuses to blur them. This is the same discipline as Wanix's existing capability story: be honest about the boundary instead of overclaiming behind it.

## The first shipped slice

The exploratory note opens with an editorial update that grounds the vision in real code (`docs/traceable-dynamic-namespaces.md:1-9`): the `#plumb` device and the `qjs-shell` mutation frame implement a concrete first slice. Shell-driven mutations and external effects flow through a traceable plumber boundary, and namespace state changes can be observed and correlated with the commands that caused them. `#plumb` is one of the first wires carrying real provenance through the system, and the browser cockpit's Activity feed is the user-facing surface of that causal model.

So the shipped story is narrow and real: a plumber topic can carry a newline-JSON envelope describing a mutation, and a reader can drain those envelopes to reconstruct *some* of the prompt-to-effect chain. The full `#trace`/`#ns`/`#volume` object model — `TraceContext`, `Mutation`, `Namespace` snapshots, commit policies like `commit=task` and `commit=agent-step` (`docs/traceable-dynamic-namespaces.md:288-323`, `381-391`) — remains a sketch. The bet behind it is that future agentic systems need more than permission prompts and final diffs; they need a live, inspectable, forkable model of causality (`docs/traceable-dynamic-namespaces.md:517-526`). The first milestone that would prove it is one convincing loop: a `gitfs` volume mounted at `/app`, a task that mutates it, a recorded trace linking task, terminal output, and the before/after namespace, then a fork of the pre-task state mounted side by side (`docs/traceable-dynamic-namespaces.md:486-500`).

## See also

- [Agents as operators](/concepts/agents-as-operators) — the actor whose work this model makes legible.
- [Approvals as files](/concepts/approvals-as-files) — the front-door trust gate; this page is the after-the-fact legibility half.
- [Wanix capsule](/concepts/wanix-capsule) — freezing a world to a portable, content-addressed snapshot, the shipped cousin of "the past as a place."
- [#plumb device](/devices/plumb) — the wire carrying the first real provenance envelopes.
- [Traceable namespaces (use case)](/use-cases/traceable-namespaces) — the same idea framed as an end-user workflow.
- [Browser cockpit](/concepts/browser-cockpit) — where the Activity feed surfaces the causal model.

## Status / honest limits

- This is an **exploratory product and architecture vision**, captured in `docs/traceable-dynamic-namespaces.md`, which states up front it is "not an ADR" (`docs/traceable-dynamic-namespaces.md:13`). Treat every `ns`/`mount`/`#trace`/`#volume` command on this page as designed-but-unshipped notation, not a current CLI.
- The only shipped slice is **`#plumb` provenance plumbing plus the `qjs-shell` mutation frame** (`docs/traceable-dynamic-namespaces.md:1-9`): some shell-driven mutations are correlated with the commands that caused them and surfaced in the cockpit Activity feed. The volume/trace/snapshot object model is not implemented.
- **Rewind is bounded by what Wanix manages.** Managed Wanix state can rewind; external effects (email, network calls, host-directory writes) can only be traced, never un-happened (`docs/traceable-dynamic-namespaces.md:272-280`). Different volume drivers would support different rewind depths.
- Live, blocking provenance over a single served 9P connection inherits the platform caveat: the serve websocket handles one 9P frame at a time per connection, so a blocking `#plumb` recv cannot interleave with a write on the same connection. End-to-end live delivery needs a second connection or concurrent frame handling.
