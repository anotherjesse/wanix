---
title: "Walkthrough 1 — Run JavaScript Outside Chrome"
slug: recipes/walkthrough-1-run-js
pageType: flow-step
oneLiner: "Build the wanix CLI once and run a real QuickJS script as a Wanix task whose four lines of output prove JavaScript runs outside the browser against a Wanix namespace."
audience: [newcomer, developer]
tags: [shipped, cli, qjs, wasi, task, namespace, walkthrough]
sourceRefs:
  - rust-walkthrough.md:57-89
  - examples/qjs-demo.js
  - examples/qjs-demo-lib.js
seeAlso:
  - concepts/qjs-task
  - concepts/everything-is-a-file
  - concepts/wasmtime-as-substrate
prerequisites: []
usedInFlows:
  - {flow: run-js-walkthrough, step: 1}
honestLimits:
  - "QuickJS runs as a WASI guest hosted by Wasmtime, not a native JS engine; it is a command-style WASI subset (no poll_oneoff readiness)."
  - "The qjs task's WASI is backed by Wanix-owned semantics; the script sees a per-task Wanix namespace, not the host disk."
  - "Files written by the demo (hello.txt) land in the task's in-memory namespace and do not persist to the host filesystem."
canonicalCaveatFor: []
---

# Walkthrough 1 — Run JavaScript Outside Chrome

**What & why.** The original Wanix lived inside Chrome. The Rust-native port moves the runtime out: QuickJS becomes a *task* hosted by Wasmtime, and the script reads its files from a Wanix namespace instead of the host disk or a browser sandbox. This first walkthrough is the smallest proof of that. You build the CLI once, run one checked-in script, and four lines of output tell you the engine, the task identity, the ES module loader, and the namespace are all real. No browser, no server, no setup beyond `cargo build`.

## 1. Build the CLI once

From the workspace root, build the native demo binary and stash its path:

```sh
cd /Users/jesse/lw/wanix
cargo build --locked --package wanix-cli
WANIX=./target/debug/wanix-rust
```

`wanix-cli` is intentionally thin — it is composition and demo plumbing over the real crates (`wanix-task`, `wanix-vfs`, `wanix-wasi`, `wanix-qjs`). The binary is named `wanix-rust`. You only build once; the rest of this walkthrough (and the others) reuse `$WANIX`.

## 2. Run the demo

```sh
$WANIX qjs examples/qjs-demo.js
```

That's the whole command. The `qjs` subcommand starts a Wanix `qjs` task, mounts your script into the task's namespace, and runs it.

## 3. Read the output

You should see exactly four lines:

```text
outside Chrome: true
task id: 1
Wanix ES module loader
hello from a Wanix namespace
```

Each line is a small, deliberate assertion. Here is the script that produced them (`examples/qjs-demo.js`):

```js
import * as std from "qjs:std";
import { runtime } from "./qjs-demo-lib.js";

const source = std.loadFile("main.js");

std.writeFile("hello.txt", "hello from a Wanix namespace");

std.out.puts("outside Chrome: " + source.includes("std.loadFile") + "\n");
std.out.puts("task id: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts(runtime + "\n");
std.out.puts(std.loadFile("hello.txt") + "\n");
std.out.flush();
```

What each line proves:

- **`outside Chrome: true`** — `std.loadFile("main.js")` read the *running script* back out of the namespace. Wanix mounts your script at the well-known path `main.js`, so the program can introspect its own source. It does, finds the string `std.loadFile`, and confirms it. No `globalThis.Wanix`, no browser API — just `qjs:std`.
- **`task id: 1`** — `std.loadFile("#task/self/id")` opened a *file* in the task service device and read this task's identity. `#task/self/id` is a path, not a syscall (more on that below).
- **`Wanix ES module loader`** — the `import { runtime } from "./qjs-demo-lib.js"` resolved a sibling ES module *through the namespace* (`examples/qjs-demo-lib.js`, which exports that one string). ES module resolution works against Wanix files, not the host filesystem directly.
- **`hello from a Wanix namespace`** — the script wrote `hello.txt` with `std.writeFile`, then read it back. The write landed inside the task's namespace and the read found it.

## 4. What happened

Under the four lines, the layering is the point:

- **QuickJS is the engine, not the kernel.** The JavaScript runs in QuickJS, which is itself a WebAssembly module hosted by **Wasmtime**. Wasmtime is the execution substrate; QuickJS is just a guest. This split is real in the crates: `wanix-qjs-engine` owns the Wasmtime/QuickJS mechanics, `wanix-qjs` is the task driver, and `wanix-wasi` provides the WASI Preview 1 syscalls — backed by Wanix-owned semantics rather than the host OS.
- **The script sees a namespace, not a disk.** `std.loadFile`, `std.writeFile`, and module imports all route through the task's Wanix namespace. `main.js` is the mounted script; `hello.txt` is a file in the same namespace. QuickJS never touches a raw host path.
- **Identity is a file.** `#task/self/id` is the task's own entry in the `#task` service device, read like any other file. Wanix — not QuickJS — owns process identity, fds, env, cwd, and exit status. The engine is a tenant.

That last point is the seed of the whole system: **everything is a file**, including the task's own metadata. You read your task id the same way you read a config file or a key/value entry. Once identity, services, and state are all files in a per-process namespace, the same contract that works locally also works across machines — which is what the mesh extends.

## Status / honest limits

This walkthrough is shipped and runs entirely locally — no browser, server, or mesh involved. A few honest framings so you know exactly what the four lines prove:

- **QuickJS is a WASI guest, not a native engine.** The JavaScript executes inside a QuickJS WebAssembly module hosted by Wasmtime. It is a command-style WASI Preview 1 subset — there is no `poll_oneoff` readiness, so this is one-shot script execution, not an event loop.
- **The namespace is per-task and in-memory.** `std.loadFile`/`std.writeFile` and module imports route through the task's Wanix namespace, not the host disk. `hello.txt` is written into that in-memory namespace and is gone when the task exits; it does not persist to your host filesystem.
- **No live LLM, key/value store, or remote mount here.** This page does not exercise the `#agent` device (whose served path is a deterministic `FakeEngine`, not a live LLM), the in-memory `#kv` store, the single-frame `serve` 9P connection, the local-trust-only exec plane, or a `/n/<peer>` mesh mount. Those caveats live on their own pages; this walkthrough is just the local qjs task.

## See also / next

- **Next:** [Walkthrough 2 — Pass process context into the task](/recipes/walkthrough-2-process-context) — feed argv, env, stdin, and cwd into a `qjs` task and watch Wanix carry them.
- [The qjs task](/concepts/qjs-task) — how a `.js` file becomes a Wanix task through `#task/new` and the `wanix-qjs` driver.
- [Everything is a file](/concepts/everything-is-a-file) — why `#task/self/id` is a path, and how that scales to the mesh.
- [Wasmtime as the substrate](/concepts/wasmtime-as-substrate) — what hosts QuickJS, and why the engine is not the kernel.
