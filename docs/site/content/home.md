---
title: "Wanix — Plan 9 Reincarnated for the Age of Agents"
slug: home
pageType: home
oneLiner: "A Rust-native, Plan 9-style OS core that runs outside Chrome: everything is a file, every process has its own namespace, and one 9P contract reaches local services and remote machines alike."
audience: [newcomer, developer, visionary]
tags: [overview, shipped, cli, mesh, agents, plan9, local-trust-only, caveat]
sourceRefs:
  - README.md
  - AGENTS.md
  - rust-walkthrough.md:57-89
  - docs/rust-vs-go-wanix.md:13-17
seeAlso:
  - everything-is-a-file
  - per-process-namespaces
  - missing-half-of-9p
  - agents-as-operators
  - js-outside-chrome
  - wire-a-mesh
prerequisites: []
usedInFlows: []
honestLimits:
  - "The served #agent uses a deterministic FakeEngine, not a live LLM; real codex is the local-trust CLI path only."
  - "#kv is in-memory service state, not durable storage; it does not survive a restart."
  - "The serve 9P websocket handles one frame at a time per connection, so a blocking read (e.g. #plumb recv) cannot be interleaved with a write on the same connection."
  - "The #cpu exec plane and real-codex #agent are local-trust-only; public/multi-user auth and per-principal namespaces are unimplemented."
  - "The shipped CLI mounts a mesh peer at the single slot /n/remote; per-peer /n/<peer> is designed but unshipped. Attach is capability-gated default-deny and live mesh peers are not portable into a capsule."
  - "The Rust port is narrower than the Go browser system; interactive shell depth, Linux/v86 9P compatibility, and mesh trust-boundary hardening are active work, not solved problems."
canonicalCaveatFor: []
---

# Wanix — Plan 9 Reincarnated for the Age of Agents

Wanix is a Rust-native operating-system core built on three Plan 9 ideas: **everything is a file**, **every process has its own namespace**, and **one protocol — 9P — connects it all**. The original Wanix proved these ideas could power a real operating environment inside the browser. The Rust port keeps that north star but moves the host boundary: Wanix becomes a native runtime, with [Wasmtime](https://wasmtime.dev) as the execution substrate and QuickJS/WASI as the first task runtime, and the browser becomes one excellent client of it rather than the foundation. The same 9P contract that exposes a local `#kv` store also imports a remote machine's namespace as local files — so devices, compute, and agents compose across nodes through a single, well-worn protocol.

## The 30-second hero

From the workspace root, run JavaScript as a native Wanix task — no Chrome, no Node:

```sh
cargo run --locked --package wanix-cli -- qjs examples/qjs-demo.js
```

```text
outside Chrome: true
task id: 1
Wanix ES module loader
hello from a Wanix namespace
```

That line is literal, not marketing. `examples/qjs-demo.js` runs in QuickJS hosted by Wasmtime; its `std.loadFile("main.js")` reads from the **task's Wanix namespace**, not the host path; `std.loadFile("#task/self/id")` reaches the task service filesystem at `#task`; and the ES import resolves through the namespace too. QuickJS is just the engine *inside* the task — Wanix owns task identity, the namespace, stdio/fds, env/cwd, and exit status (`rust-walkthrough.md:57-89`).

## The thesis, in one line

> **Everything is a file, every process has its own namespace, and one 9P contract reaches local services and remote machines alike.**

A "file" here is an interface, not a blob on disk: `#kv/<key>` reads and writes a value; `#pipe/new` allocates a byte channel; `#agent/<id>/prompt` drives an LLM session. A "namespace" is a per-process view assembled by binding those file trees where you want them — your view of the world is composable and confined. And one filesystem contract reaches everything: the same `mount` operation that binds a local export binds a peer's namespace over QUIC (today at the single slot `/n/remote`; per-peer `/n/<peer>` is the designed shape). There is no second API for "remote."

## Pick your on-ramp

- **"I want to run JS/WASI outside Chrome."** Start with the [js-outside-chrome flow](/learn/js-outside-chrome). `qjs` runs JavaScript and `wasm` runs compiled `wasm32-wasi` modules as a second task runtime on the same substrate, sharing one filesystem.
- **"I want to wire machines together."** Go to [wire-a-mesh](/learn/wire-a-mesh) and [The Missing Half of 9P](/concepts/missing-half-of-9p). Each node carries a persisted ed25519 identity; attach is capability-gated (default-deny); a peer mounts at `/n/remote` today (`/n/<peer>` is the designed per-peer shape), and every device imports across the mesh for free.
- **"I'm extending the core."** See [add-a-service-device](/learn/add-a-service-device) and [add-driver-or-transport](/learn/add-driver-or-transport). Each service device is a plain `FileSystem` — that is why `#kv`/`#cas`/`#agent` work locally *and* across the mesh with no extra code.
- **"I want the deep idea."** Take the [plan9-ideas-tour](/learn/plan9-ideas-tour): [everything-is-a-file](/concepts/everything-is-a-file), [per-process-namespaces](/concepts/per-process-namespaces), and [agents-as-operators](/concepts/agents-as-operators) — an agent that operates your machine through the same files you do.

## What Wanix is (and is not)

Wanix **is** a native microkernel-style runtime: it owns the POSIX-ish process model, per-process namespaces for isolation, the WASI filesystem semantics tasks see, and the 9P server/client that joins it all. The browser cockpit (a Code OSS / VS Code web extension under `workbench/`) is a **client** — a human operator surface that drives the served namespace over direct 9P. It is a frontend, not the runtime foundation (`docs/rust-vs-go-wanix.md:13-17`).

Wanix is **not** finished, and we say so plainly. The Go version is still broader as a browser system; the Rust port is narrower but deeper at the native runtime boundary. Interactive shell depth, broad Linux/v86 9P compatibility, and hardening the mesh trust boundary (per-principal namespaces, grant lifecycle) are active work, not solved problems (`AGENTS.md`).

## Quickstart

The first command compiles the workspace (a few minutes); subsequent runs are instant. The full build story is on [build & install](/reference/build-and-install).

```sh
# 1. Run JavaScript outside Chrome as a native qjs task
cargo run --locked --package wanix-cli -- qjs examples/qjs-demo.js

# 2. Drive a terminal-backed qjs task
cargo run --locked --package wanix-cli -- \
  qjs-term --stdin "hello terminal" examples/qjs-term-demo.js

# 3. A filesystem shell over the #term device
printf 'write note.txt hello\nls\ncat note.txt\nexit\n' \
  | cargo run --locked --package wanix-cli -- qjs-shell

# 4. Serve the browser cockpit with all service devices (open the printed URL).
#    serve needs an existing root directory — create it first.
mkdir -p /tmp/wanix-root
cargo run --locked --package wanix-cli -- \
  serve --root /tmp/wanix-root --bundle workbench-fs9p --wanix-services
```

The cockpit (step 4) inspects the service devices over 9P, repairs a broken program through `#agent`, runs a qjs→wasm→qjs duet on one shared filesystem, and serves HTTP apps at `/.wanix/app/<name>` with `#kv`-backed state (`README.md`).

## Status / honest limits

So the hero demos read straight, here is what is and is not behind them:

- **The served `#agent` is a deterministic `FakeEngine`, not a live LLM.** The cockpit's agent-repair demo runs against a scripted engine; real codex is the local-trust CLI path (`wanix agent`) only.
- **`#kv` is in-memory service state, not durable storage.** It backs app/agent state for a running node and does not survive a restart.
- **The serve 9P websocket handles one frame at a time per connection.** A blocking read (e.g. `#plumb/<topic>/recv`) cannot be interleaved with a write on the same connection — live pub/sub needs a second connection.
- **The `#cpu` exec plane and real-codex `#agent` are local-trust-only.** Public/multi-user auth and per-principal namespaces are not implemented; the mesh trust boundary (grant lifecycle) is active work.
- **The shipped CLI mounts a mesh peer at one slot, `/n/remote`.** Per-peer `/n/<peer>` paths are designed but unshipped. Attach is capability-gated (default-deny); live mesh peers and ephemeral handles are not portable into a `.wcap` capsule.

The Rust port is narrower than the Go browser system but deeper at the native runtime boundary; the gaps above are tracked work, not hidden ones (`AGENTS.md`).

## See also / next

- **Learn** the flows: [js-outside-chrome](/learn/js-outside-chrome) · [wire-a-mesh](/learn/wire-a-mesh) · [http-app-with-kv](/learn/http-app-with-kv)
- **Concepts**: [everything-is-a-file](/concepts/everything-is-a-file) · [per-process-namespaces](/concepts/per-process-namespaces) · [missing-half-of-9p](/concepts/missing-half-of-9p) · [agents-as-operators](/concepts/agents-as-operators)
- **Devices**: the [#task](/devices/task), [#term](/devices/term), [#kv](/devices/kv), [#pipe](/devices/pipe), [#plumb](/devices/plumb), [#cas](/devices/cas), and [#agent](/devices/agent) reference — each page is that device's file contract; consult one when a flow you are following touches it
- **Find** anything: jump to the [reference index](/reference/adr-index) or the [find index](/find/index)
