---
title: Use cases
slug: use-cases/index
pageType: overview
oneLiner: Outcome-first answers to "what is this for?" — each ending in exactly one recipe you can run.
audience: [newcomer, visionary]
tags: [use-case]
sourceRefs: []
seeAlso: [use-cases/agents-on-your-files, use-cases/browser-cockpit, use-cases/distributed-dev-environments, use-cases/personal-compute-mesh, use-cases/portable-worlds, use-cases/traceable-namespaces]
prerequisites: []
usedInFlows: []
honestLimits: []
canonicalCaveatFor: []
---

# Use cases

Outcome-first answers to "what is this for?" — each ending in exactly one recipe you can run.

Use cases lead with the outcome, name the concepts it rests on, and hand you a single recipe to make it real. They are the bridge from "why would I want this" to "here, run this".

## Pages

- [**AI Agents Operating on Your Files Across Devices**](/use-cases/agents-on-your-files) — #agent repairs a broken program with approvals-as-files; over the mesh it reaches a peer's #kv/#cas/files; two agents on two machines collaborate via #plumb.
- [**A Browser-Native Operator Cockpit**](/use-cases/browser-cockpit) — A Code OSS / VS Code web extension drives the whole namespace over direct 9P — inspect the service devices, run the agent repair demo, run a qjs->wasm->qjs duet on one shared FS, serve HTTP apps backed by #kv, and self-check the device set.
- [**Distributed Dev Environments**](/use-cases/distributed-dev-environments) — #cpu runs your task on the node holding the source against its fast local namespace, with the output returning over the wire — Plan 9 cpu(1) over the open internet, behind a default read-only jail.
- [**Your Personal Compute Mesh**](/use-cases/personal-compute-mesh) — Run a Wanix node on your laptop, phone, and a cloud box, then mount any one of them as local files over iroh QUIC — devices and all — gated by per-peer ed25519 grants.
- [**Portable Worlds via Capsules**](/use-cases/portable-worlds) — wanix capsule save freezes a whole world — the directory an agent built — into a CAS-backed, BLAKE3-verified, deduplicated set of blobs, and hand someone one hash and they materialize the entire world, verifying every blob.
- [**Traceable, Forkable, Rewindable Namespaces (Exploratory)**](/use-cases/traceable-namespaces) — An agentic runtime should make the chain prompt->plan->command->task->log->mutation->result visible, addressable, and forkable, so you can ask "what changed, and why?" and rewind or fork the world.

## See also

- [Learn — guided flows](/learn/index)
- [Concept index](/find/concepts)
- [Search & glossary](/find/index)
