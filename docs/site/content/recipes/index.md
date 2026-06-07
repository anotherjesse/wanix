---
title: Recipes
slug: recipes/index
pageType: overview
oneLiner: Copy-paste transcripts that produce a real result, each linking up to the use case it serves.
audience: [newcomer, developer]
tags: [recipe]
sourceRefs: []
seeAlso: [recipes/00-scaffold-a-project, recipes/01-repair-broken-qjs, recipes/02-mount-remote-peer, recipes/03-freeze-world-to-capsule, recipes/04-tiny-http-app-with-kv, recipes/05-two-agents-collaborate, recipes/walkthrough-1-run-js, recipes/walkthrough-2-process-context]
prerequisites: []
usedInFlows: []
honestLimits: []
canonicalCaveatFor: []
---

# Recipes

Copy-paste transcripts that produce a real result, each linking up to the use case it serves.

Every recipe is a verbatim shell transcript with expected output — paste it, watch it work, then follow the link up to the use case that explains *why*. Recipes are the "how"; use cases are the "why".

## Pages

- [**Scaffold a project with wanix new**](/recipes/00-scaffold-a-project) — Generate a JS or Rust Wanix project with the guest SDK wired in.
- [**Recipe 01 — Repair a Broken qjs Program with an Agent**](/recipes/01-repair-broken-qjs) — Hand a crashing qjs program to a Wanix-backed agent from inside the cockpit and watch it propose a patch, ask for approval, and write the fix back — all as plain reads and writes on the #agent service.
- [**Recipe 02 — Mount a Remote Wanix Peer and Run a Job on It**](/recipes/02-mount-remote-peer) — Stand up two nodes, have A dial B over iroh QUIC, mount B's directory as a Plan 9 namespace, and send a #cpu job so the code runs on B against B's local data.
- [**Recipe 03 — Freeze a Wanix World to a Portable Capsule**](/recipes/03-freeze-world-to-capsule) — Freeze a directory the agent (or you) just built onto the content-addressed plane so the same world can be reconstructed deterministically anywhere the same capsule id is reachable.
- [**Recipe 04 — A Tiny HTTP App with #kv-Backed Counter State**](/recipes/04-tiny-http-app-with-kv) — A single-file qjs handler at apps/counter.js increments a per-request counter held in #kv — nothing about the state lives in the handler, only in the device.
- [**Recipe 05 — Two Agents Collaborate via #agent/<id>/reply**](/recipes/05-two-agents-collaborate) — Agent A drives a rename refactor, spawns a second session B with a sub-goal, blocks on B's reply (single-read, EOF-terminating), and integrates the findings — all as files.
- [**Walkthrough 1 — Run JavaScript Outside Chrome**](/recipes/walkthrough-1-run-js) — Build the wanix CLI once and run a real QuickJS script as a Wanix task whose four lines of output prove JavaScript runs outside the browser against a Wanix namespace.
- [**Walkthrough 2 — Feed env, cwd, stdin, and argv into a Script**](/recipes/walkthrough-2-process-context) — Pass --env/--cwd/--stdin and -- argv and watch scriptArgs, std.getenv, fd 0, and #task/self/id flow in as task state — no globalThis.Wanix bridge.

## See also

- [Learn — guided flows](/learn/index)
- [Concept index](/find/concepts)
- [Search & glossary](/find/index)
