---
title: JavaScript outside Chrome in 10 minutes
slug: learn/js-outside-chrome
pageType: flow
oneLiner: "A guided first-win track: run JS as a Wanix task, feed it env/stdin/argv, mount a host dir, then meet the cockpit."
audience: [newcomer]
tags: [learn, qjs, cli, shipped, local-trust-only, caveat]
sourceRefs:
  - rust-walkthrough.md
  - docs/recipes/01-repair-broken-qjs.md
  - docs/recipes/04-tiny-http-app-with-kv.md
  - crates/wanix-cli/src/serve/http/app.rs:16-44
  - workbench/src/web/http-app-demo.ts:15
  - workbench/Makefile:1-25
seeAlso:
  - concepts/everything-is-a-file
  - concepts/per-process-namespaces
  - concepts/qjs-task
  - concepts/qjs-shell
  - concepts/service-devices
  - concepts/wanix-backed-wasi
  - recipes/walkthrough-1-run-js
  - learn/http-app-with-kv
  - learn/wire-a-mesh
prerequisites: []
usedInFlows: []
honestLimits:
  - "The cockpit needs a one-time vscode-web build (needs Go); the CLI fallback reaches the same payoff."
  - "The served #agent is a deterministic FakeEngine, not a live LLM."
  - "#kv is in-memory: counter state lives only as long as the serve process."
canonicalCaveatFor: []
---

# JavaScript outside Chrome in 10 minutes

A guided first-win track: run JS as a Wanix task, feed it env/stdin/argv, mount a host dir, then meet the cockpit.

Wanix was born in the browser, but the Rust-native port runs the same ideas on a bare machine — no Chrome, no `v86`, no WebAssembly host page. This flow takes a newcomer from one command (`wanix-rust qjs file.js`) to a script that reads its environment, mounts a host directory, and finally to the browser cockpit driving the same namespace. Each step shows the visible result first, then names the Plan 9 idea underneath it. Budget about ten minutes.

## Build once

Everything below uses the native CLI. Build it from the workspace root, then alias the binary so the commands read cleanly:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

The first build compiles the workspace (around twenty-two crates), so it is slow once and instant after. `wanix-rust --help` lists the subcommands you will use: `qjs`, `qjs-term`, `qjs-shell`, and `serve` (`rust-walkthrough.md:38-51`).

## Hero: JavaScript with no browser

```sh
wanix-rust qjs examples/qjs-demo.js
```

```text
outside Chrome: true
task id: 1
Wanix ES module loader
hello from a Wanix namespace
```

That `outside Chrome: true` is the whole north star in one line. QuickJS executed the script, but QuickJS is only the *engine inside* the task — Wanix owns everything around it (`rust-walkthrough.md:57-88`).

## What just happened, jargon-free

Three things, none of them browser glue:

- **The script is a task, not a process.** `task id: 1` is Wanix's own identity for the running script, readable as a file at `#task/self/id`. The QuickJS interpreter does not know its own id; Wanix assigned it.
- **Reads and writes hit a namespace, not the host.** `std.loadFile("main.js")` read from the task's [per-process namespace](/concepts/per-process-namespaces), not from a host path. The same call reached `#task/self/id` — a [service device](/concepts/service-devices) file — and loaded an ES module, all through one resolver.
- **It is files all the way down.** The script's own identity, its modules, and its data are all just files in a namespace it composes for itself. That is the [everything-is-a-file](/concepts/everything-is-a-file) idea doing real work.

The split is deliberate: QuickJS handles JavaScript; Wanix owns task identity, the namespace, fds, and exit status (`rust-walkthrough.md:84-88`). That is what makes it a [qjs task](/concepts/qjs-task) and not "a JS process."

## Feed it env, cwd, stdin, and argv

Real programs read their context. A `qjs` task gets cwd, environment, stdin on fd 0, and script arguments — all from Wanix, all live (`rust-walkthrough.md:90-139`):

```sh
wanix-rust qjs \
  --env MODE=walkthrough \
  --cwd app \
  --stdin "hello fd0" \
  examples/qjs-demo.js \
  -- alpha "two words"
```

Inside the script, `scriptArgs` carries the argv, `std.getenv("MODE")` reads the environment, `os.read(0, ...)` reads the stdin string, and `std.loadFile("#task/self/id")` reads its task id. None of this is a `globalThis.Wanix` helper — that browser-era bridge is retired. Guest code uses ordinary `qjs:std`, `qjs:os`, `scriptArgs`, and service files (`rust-walkthrough.md:136-139`).

## A short shell step: files all the way down

Pipe a few commands into the bundled QuickJS shell to see the namespace as a place you can poke:

```sh
printf 'write note.txt hello\nls\ncat note.txt\nexit\n' | wanix-rust qjs-shell
```

```text
shell task: 1
$ wrote note.txt
$ note.txt
$ hello
$ bye
```

`write`, `ls`, and `cat` are built-in commands operating on the shell task's namespace (`rust-walkthrough.md:336-353`). The shell itself is a `qjs` task whose fds 0/1/2 are wired through a terminal device; the guest shell owns echo, editing, and command dispatch (`rust-walkthrough.md:355-359`). This is the [qjs-shell](/concepts/qjs-shell) contract — terminal behavior is a Wanix device, not browser xterm glue.

## Mount a host directory explicitly

Wanix does not give a task ambient access to your disk. Authority is *bound in* by name. First create a host file:

```sh
mkdir -p /tmp/wanix-host && echo "from host" > /tmp/wanix-host/input.txt
```

Then mount that directory into the task's namespace under the name `host`:

```sh
wanix-rust qjs --mount /tmp/wanix-host=host examples/qjs-host-mount.js
```

```text
host input: from host
host output: mounted output for from host
```

The script read `host/input.txt` and wrote `host/output.txt` — and that write landed back on disk at `/tmp/wanix-host/output.txt`. The CLI built a rooted host filesystem and bound it at `host`; the task only ever sees a namespace path, while the host-root boundary holds (`rust-walkthrough.md:142-178`). Host files are explicit authority, not ambient.

## Boot the cockpit (or take the CLI fallback)

The browser cockpit is a VS Code / Code OSS web extension that drives this same namespace over direct 9P. It needs a one-time build that fetches vscode-web and runs a Go build step (`workbench/Makefile:1-25`):

```sh
cd workbench && make build   # one-time; requires Go on PATH
wanix-rust serve --wanix-services --bundle workbench-fs9p --listen 127.0.0.1:7654 .
# open http://127.0.0.1:7654/?bundle=workbench-fs9p
```

No Go handy? Skip the cockpit. The CLI fallback reaches the same payoff — the cockpit is a thin client, not the runtime:

```sh
wanix-rust agent --fake "Repair broken.js so it writes out/result.txt"
```

`--wanix-services` binds the service devices (`#task`, `#term`, `#kv`, `#agent`, and friends) into the served namespace (`docs/recipes/01-repair-broken-qjs.md:57-91`).

## Click-through, with the file-ops reveal

Each cockpit action is just reads and writes on service files:

- **Agent Repair** allocates a session by reading `#agent/new`, writes a prompt to `#agent/<id>/prompt`, watches `#agent/<id>/events`, and approves a parked edit by writing `approve <req>` to `#agent/<id>/ctl` (`docs/recipes/01-repair-broken-qjs.md:124-195`). The entire LLM contract reduces to read/write on one tree.
- **Duet** runs a `qjs` task and a compiled `wasm` task against one shared filesystem, proving two task tiers on one substrate.
- **HTTP counter** serves a handler at `/.wanix/app/<name>` (loopback-only, services-gated) whose state is `#kv/http-counter` (`crates/wanix-cli/src/serve/http/app.rs:16-44`, `workbench/src/web/http-app-demo.ts:15`).
- **Self-check** probes the device set over 9P.

## Write your own app

Drop a one-file handler at `apps/counter.js` that reads `#kv/http-counter`, increments, and writes it back — no state lives in the code (`docs/recipes/04-tiny-http-app-with-kv.md:36-99`). Run it repeatedly against a running `serve --wanix-services` and watch the counter climb. The full build is the [HTTP-app-with-kv flow](/learn/http-app-with-kv) and [recipe 04](/recipes/04-tiny-http-app-with-kv).

## Where next

You have run JavaScript as a real Wanix task with live WASI, context, and an explicit host mount. The natural next step is to take this same namespace across machines: see [wire a mesh](/learn/wire-a-mesh), where importing a remote peer's files is the *same* bind operation you just used for `/tmp/wanix-host`.

## See also

- [Everything is a file](/concepts/everything-is-a-file) · [per-process namespaces](/concepts/per-process-namespaces) · [service devices](/concepts/service-devices)
- [The qjs task](/concepts/qjs-task) · [qjs-shell](/concepts/qjs-shell) · [Wanix-backed WASI](/concepts/wanix-backed-wasi)
- Recipe: [run JS](/recipes/walkthrough-1-run-js) · Flows: [HTTP app with #kv](/learn/http-app-with-kv) · [wire a mesh](/learn/wire-a-mesh)

## Status / honest limits

- The cockpit requires a one-time vscode-web build with Go on PATH (`workbench/Makefile:1-25`). The CLI fallback reaches the same outcome; the cockpit is a 9P client, not the runtime.
- The served `#agent` uses a deterministic `FakeEngine`, not a live LLM (`docs/recipes/01-repair-broken-qjs.md:88-91`). The real codex engine is the local-trust `wanix agent` CLI path only.
- `#kv` is an in-memory tier: the counter survives between calls only while the serve process is alive (`docs/recipes/04-tiny-http-app-with-kv.md:86-99`). Freeze a world to a capsule for durability.
- The HTTP-app route at `/.wanix/app/<name>` is loopback-only and requires `--wanix-services` (`crates/wanix-cli/src/serve/http/app.rs:34-44`).
