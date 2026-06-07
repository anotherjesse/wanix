---
title: A Browser-Native Operator Cockpit
slug: use-cases/browser-cockpit
pageType: use-case
oneLiner: A Code OSS / VS Code web extension drives the whole namespace over direct 9P — inspect the service devices, run the agent repair demo, run a qjs->wasm->qjs duet on one shared FS, serve HTTP apps backed by #kv, and self-check the device set.
audience: [visionary, developer]
tags: [cockpit, mesh, shipped, local-trust-only, caveat]
sourceRefs:
  - docs/integration/STATUS.md:432-475
  - docs/integration/cockpit-meets-mesh.md:315-431
  - workbench/src/web/extension.ts:15
  - crates/wanix-cli/src/serve/http/app.rs:35-64
  - README.md:51-57
seeAlso:
  - concepts/browser-cockpit
  - concepts/direct-9p-operator-surface
  - concepts/http-app-route
  - concepts/service-devices
  - concepts/wanix-as-host
  - use-cases/personal-compute-mesh
  - recipes/01-repair-broken-qjs
  - recipes/04-tiny-http-app-with-kv
prerequisites:
  - concepts/browser-cockpit
  - concepts/service-devices
usedInFlows: []
honestLimits:
  - "The served #agent is a deterministic FakeEngine, not a live LLM; real codex is the local-trust 'wanix agent' CLI path only."
  - "#kv is in-memory: state lives only as long as the serve process; freeze to a capsule to persist."
  - "The cockpit's #plumb self-check probes the publish path only — serve handles one 9P frame at a time per connection, so a blocking recv can't interleave with a write on the same connection."
  - "The bundle is still named --bundle workbench-fs9p; --bundle cockpit is a planned rename."
  - "No mesh panel yet: verified peers, mount/revoke, and browser-as-peer are designed but unshipped; the cockpit operates one local node."
  - "The HTTP-app route at /.wanix/app/<name> is loopback-only and requires --wanix-services."
  - "v86-shared-demo is still a STUB; its sidebar row does nothing yet."
---

# A Browser-Native Operator Cockpit

A Code OSS / VS Code web extension drives the whole namespace over direct 9P — inspect the service devices, run the agent repair demo, run a qjs->wasm->qjs duet on one shared FS, serve HTTP apps backed by #kv, and self-check the device set.

**What & why.** The Rust runtime is a pile of file-shaped devices — `#task`, `#term`, `#agent`, `#kv`, `#pipe`, `#plumb`, `#cas` — bound under one namespace and served over 9P. That is a great machine and a terrible thing to operate by hand. The cockpit is the human surface: a Code OSS / VS Code web extension (it lives under `workbench/`) that attaches to a served namespace over direct 9P and gives an operator a sidebar tree, a device inspector, a terminal, and a row of one-click demos. The trick is that the cockpit invents *no* control protocol. Every button it offers is a 9P read or write against a device file. Inspecting `#agent` and inspecting `#task` use the same classifier; running a demo means allocating a task and writing `start` to its `ctl`. The operator UI is built on exactly the boundaries the runtime enforces (`docs/integration/cockpit-meets-mesh.md:418`).

## The outcome: a human surface for the file-shaped runtime

Picture VS Code in a browser tab, but the explorer is a live 9P filesystem and a new **Wanix** entry sits in the activity bar. Open it and the **WANIX: SYSTEM** tree populates from the served namespace — drivers, tasks, terminals, agents, data stores, checks, activity, routes. The terminal pane shows a live `qjs-shell` (`shell task: 1`, a `$` prompt) read straight from the `#task` device, not from extension memory (`docs/integration/STATUS.md:399-404`). Nothing here is a mock. The tree rows are the bytes of the devices; closing the tab does not change what the runtime is doing, because the cockpit is a client, not the runtime.

This is the same "everything is a file" model you already met in the CLI, just rendered. Where the CLI is the hero loop, the cockpit is the dashboard for [Wanix as host](/concepts/wanix-as-host) — the [browser cockpit](/concepts/browser-cockpit) reading the [service devices](/concepts/service-devices) through the [direct-9P operator surface](/concepts/direct-9p-operator-surface).

## Boot it

The cockpit is one serve bundle. Build the CLI, alias it, then serve the workbench bundle with the service devices enabled:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

wanix-rust serve --root /tmp/wanix-root \
    --bundle workbench-fs9p --wanix-services
```

Open the printed URL and the Code OSS shell boots, registers `wanix:` as a filesystem provider over direct 9P, and registers the system view and the `wanix-inspect:` handler (`README.md:51-57`, `docs/integration/cockpit-meets-mesh.md:329-337`). `--wanix-services` is the load-bearing flag: without it the served namespace is just the root, and the device rows and the HTTP-app route below are absent.

Note the bundle is still spelled `--bundle workbench-fs9p`. A rename to `--bundle cockpit` is planned but unshipped (`docs/integration/STATUS.md:114`); use the current name.

## The five live-wired features

The cockpit's payoff is five sidebar actions, each verified end-to-end against the live devices over direct 9P (`docs/integration/STATUS.md:432-470`). The point of each is the same reveal: *the fancy operation is just file ops.*

**Inspect the service devices.** The Namespace tree lists every bound device. The set is not hard-coded; it comes from the discovery document's `services.devices`, sourced from one place in the Rust serve (`roots::INSPECTABLE_SERVICE_DEVICES`). Open a device and the inspector renders its live directory over 9P with allocator/control/stream annotations — `#kv` (empty until you write), `#agent` (`new` is the allocator), `#cas` (`have/` probe plus `ingest`). The `new` file is treated as an unsafe link and never auto-read, because reading an allocator file *allocates* (`docs/integration/STATUS.md:438-443`). What looks like a device browser is `walk` + `read` against a `FileSystem`.

**Run the agent repair demo.** One click opens a `#agent` session, submits a patch plan as an `approve:` prompt, polls `pending`, resolves the approval via `ctl`, then applies the approved patch. The visible result: `/agent/broken.js` is fixed, `result.txt` reads "FIXED BY WANIX AGENT", and the report logs the session and the approval (`docs/integration/STATUS.md:444-449`). The Plan 9 term arrives after the effect: the approval was a file write, not an API call — see [approvals as files](/concepts/approvals-as-files). The engine driving this is deterministic, not a live model; that is the first honest limit below.

**Run the qjs->wasm->qjs duet.** Three tasks on one shared filesystem: a qjs producer writes a value, a compiled `wasm32-wasi` task transforms it, a qjs verifier reads the result. The visible proof is `shared/out.txt` reading `rust-wasm saw: hello from qjs duet` (`docs/integration/STATUS.md:457-459`). Two task tiers — interpreted and compiled — observe the same `MemFs` state, which is [two tiers, one substrate](/concepts/two-tiers-one-substrate) made clickable.

**Serve an HTTP app backed by #kv.** The cockpit restores the Rust route `/.wanix/app/<name>`: the serve process resolves `apps/<name>.js` (or `.wasm`) under the served root, runs it as a task, and returns its stdout. The shipped counter handler does read-modify-write on the `#kv/http-counter` key, so `curl` and the cockpit increment *one shared counter* (`docs/integration/STATUS.md:461-466`). The route is gated twice in the Rust serve: it requires `--wanix-services`, and it refuses any non-loopback client (`crates/wanix-cli/src/serve/http/app.rs:55-64`). WASI guests can reach `#kv` from the namespace root regardless of cwd, which is what lets the handler open `#kv/http-counter` at all. See the [HTTP-app route](/concepts/http-app-route) and [recipe 04](/recipes/04-tiny-http-app-with-kv).

**Self-check the device set.** One action probes drivers, services, shell, report storage, and the devices `#agent`/`#kv`/`#pipe`/`#cas`/`#plumb`, then writes `/.wanix/cockpit-check.{md,json}`. With the bridge live-stream fix and `O_WRONLY` write ends, `#pipe` and `#plumb` no longer hang and the check completes green (`docs/integration/STATUS.md:451-456`). One caveat shapes the `#plumb` probe — covered below.

## See also

- Concepts: [browser cockpit](/concepts/browser-cockpit) · [direct-9P operator surface](/concepts/direct-9p-operator-surface) · [HTTP-app route](/concepts/http-app-route) · [service devices](/concepts/service-devices) · [Wanix as host](/concepts/wanix-as-host)
- Devices: [#agent](/devices/agent) · [#kv](/devices/kv) · [#task](/devices/task) · [#cas](/devices/cas)
- Recipes: [Repair a broken qjs program with #agent](/recipes/01-repair-broken-qjs) · [A tiny HTTP app backed by #kv](/recipes/04-tiny-http-app-with-kv)
- Next use case: [Your personal compute mesh](/use-cases/personal-compute-mesh) — the CLI hero loop the cockpit will eventually surface through a mesh panel.

## Status / honest limits

This is real and runnable on this branch, verified in headless Chromium and via `curl`. Be precise about the edges:

- **The served `#agent` is a deterministic FakeEngine, not a live LLM.** The repair demo's fix is scripted, not reasoned. Real codex runs only on the local-trust `wanix agent` CLI path, never on the served `#agent` (`docs/integration/cockpit-meets-mesh.md:119`). See [FakeEngine vs codex](/concepts/fakeengine-vs-codex).
- **`#kv` is in-memory.** The HTTP counter survives across requests only because the value lives in the serve process's `KvDevice` map; the count resets when the serve process exits. To persist a world, freeze it to a [Wanix capsule](/concepts/wanix-capsule).
- **`serve` handles one 9P frame at a time per connection.** A blocking `#plumb/<topic>/recv` cannot interleave with a write on the same connection, so the self-check probes the publish path only — it does not verify end-to-end delivery (`docs/integration/STATUS.md:471-475`). Live cross-connection pub/sub wants a second 9P connection. See the [single-frame serve caveat](/concepts/single-frame-serve-caveat).
- **Exec devices are local-trust only.** `#task`, `#agent`, and `#cpu` are not exposed to untrusted or public peers. The cockpit's reach is "cheap, scalable isolation," not "safe for arbitrary untrusted code," and there are no hard CPU/memory limits yet. The HTTP-app route is loopback-gated for exactly this reason (`crates/wanix-cli/src/serve/http/app.rs:61-64`). See [loopback-only handoffs](/concepts/loopback-only-handoffs).
- **No mesh panel, no browser-as-peer yet.** The cockpit operates one local node. A verified-peers panel with mount/revoke and browser-as-peer attach is designed (`docs/integration/cockpit-meets-mesh.md:178-215`) but unshipped; the mesh is driven from the CLI for now.
- **The bundle is still `--bundle workbench-fs9p`,** and `v86-shared-demo` is still a stub — its row appears but does nothing (`workbench/src/web/extension.ts:15`).

None of these undercut the core claim. What ships today is an operator UI that walks the real device set over direct 9P, inspects allocators safely, repairs a program through `#agent`, runs the compiled-and-interpreted duet on one shared filesystem, serves a `#kv`-backed HTTP counter, and self-checks the whole set — every action a file read or write, no side channel.

## Runnable path

The fastest visible win is [recipe 01 — Repair a broken qjs program with #agent](/recipes/01-repair-broken-qjs), which exercises the inspector, the duet starters, and the agent demo end to end. For the HTTP-app loop, walk [recipe 04 — A tiny HTTP app backed by #kv](/recipes/04-tiny-http-app-with-kv): scaffold `apps/counter.js`, then increment `#kv/http-counter` from both `curl` and the cockpit and watch one shared number climb.
