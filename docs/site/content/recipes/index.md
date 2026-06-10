---
title: Recipes
slug: recipes/index
pageType: overview
oneLiner: Copy-paste transcripts that produce a real result, each linking up to the use case it serves.
audience: [newcomer, developer]
tags: [recipe]
sourceRefs: []
seeAlso: [recipes/00-scaffold-a-project, recipes/01-repair-broken-qjs, recipes/02-mount-remote-peer, recipes/03-freeze-world-to-capsule, recipes/04-tiny-http-app-with-kv, recipes/05-two-agents-collaborate, recipes/06-compose-volume-and-tools, recipes/07-chatroom-over-the-mesh, recipes/08-real-tools-with-config, recipes/09-web-door-gateway, recipes/10-name-your-world, recipes/walkthrough-1-run-js, recipes/walkthrough-2-process-context]
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
- [**Recipe 06 — Compose a Volume and Two Tools in One Shell Across the Mesh**](/recipes/06-compose-volume-and-tools) — Serve a demo-notes volume and the upper/sha256 tools as three mesh resources, mount all three into one wanix-rust sh namespace, pipe a remote file through a remote tool back into the remote volume, then drive the raw job protocol with no sugar.
- [**Recipe 07 — A Chatroom That Is a Mounted Program**](/recipes/07-chatroom-over-the-mesh) — app-serve the bundled qjs chatroom with a durable --state dir, post from two principals, claim nicks that render but never authenticate, watch live delivery with mount-cat --follow, defeat a payload impersonation, then kill and restart the room with its history and nicks intact.
- [**Recipe 08 — Your Own Programs as Mesh Tools (tools.toml)**](/recipes/08-real-tools-with-config) — Write a tools.toml that wraps real host programs (tr, sort, your own script), serve each as its own mesh tool, run jobs from the shell with the tool builtin, watch a job's events stream live with mount-cat --follow, then hit a real timeout and abort a real run.
- [**Recipe 09 — The Web Door (named origins over one gateway)**](/recipes/09-web-door-gateway) — Bind the chat webapp and its mesh room under http://chat.localhost:PORT and a static docs site under a second name, drive posts/latest/SSE with curl, watch an outage map to 503, and read the one identity caveat before demoing.
- [**Recipe 10 — Name Your World (catalog, names, recipes)**](/recipes/10-name-your-world) — Register a volume, a tool, and the chatroom in the catalog with --register, watch liveness in catalog ls, mount everything by bare name, run the room's verbs, save the whole desk as a recipe, and rebuild it on a fresh machine from two copied directories.
- [**Walkthrough 1 — Run JavaScript Outside Chrome**](/recipes/walkthrough-1-run-js) — Build the wanix CLI once and run a real QuickJS script as a Wanix task whose four lines of output prove JavaScript runs outside the browser against a Wanix namespace.
- [**Walkthrough 2 — Feed env, cwd, stdin, and argv into a Script**](/recipes/walkthrough-2-process-context) — Pass --env/--cwd/--stdin and -- argv and watch scriptArgs, std.getenv, fd 0, and #task/self/id flow in as task state — no globalThis.Wanix bridge.

## See also

- [Learn — guided flows](/learn/index)
- [Concept index](/find/concepts)
- [Search & glossary](/find/index)
