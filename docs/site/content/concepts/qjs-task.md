---
title: qjs Task — JavaScript Outside Chrome
slug: concepts/qjs-task
pageType: concept
oneLiner: QuickJS compiled to a WASI reactor on Wasmtime runs a .js file as a first-class Wanix task with a live namespace, fds, argv/env/cwd, and an observable exit status.
audience: [newcomer, developer]
tags: [tasks, wasi, quickjs, shipped, cli]
sourceRefs:
  - crates/wanix-qjs/src/driver.rs:82-102
  - crates/wanix-qjs/src/runner.rs:196-263
  - crates/wanix-qjs/src/lib.rs:1-43
  - crates/wanix-qjs/src/lib.rs:127-175
  - crates/wanix-qjs/src/host_api.rs:14-27
  - crates/wanix-qjs-engine/docs/architecture.md:19-23
sourceRefs_note: line ranges are anchors, not exhaustive
seeAlso:
  - concepts/compiled-wasm-task-driver
  - concepts/two-tiers-one-substrate
  - concepts/guest-js-guardrails
  - concepts/quickjs-snapshots-are-vm-images
  - concepts/bounded-execution-policy
  - devices/task
prerequisites:
  - concepts/tasks-own-process-identity
  - concepts/wasmtime-as-substrate
usedInFlows:
  - {flow: js-outside-chrome, step: 2}
honestLimits:
  - The bounded-execution knobs (interrupt-poll budget, heap limit, event-loop wait budget) are host policy applied per run, not a scheduler, signals, or a general task-cancellation model.
  - A qjs task runs to completion synchronously on the calling thread; there is no preemption.
  - The WASI surface is a command-style subset; poll_oneoff readiness is not the model, and async timers only fire within an explicit wait budget.
canonicalCaveatFor: []
---

# qjs Task — JavaScript Outside Chrome

QuickJS compiled to a WASI reactor on Wasmtime runs a `.js` file as a first-class Wanix task with a live namespace, fds, argv/env/cwd, and an observable exit status.

A `.js` file is not a script you shell out to. It is a *task* — a thing Wanix allocates, starts, and watches, with its own private file view and its own exit code. The JavaScript engine that runs it is QuickJS, but QuickJS does not get to touch your real machine: it is a WebAssembly module on Wasmtime, and every file it opens, every byte it prints, and every environment variable it reads is supplied by Wanix. The result is JavaScript that runs *outside Chrome*, with real file plumbing but no ambient authority.

## The command, then the name

Build the CLI and run a script:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'
wanix qjs examples/qjs-demo.js
```

What ran is a Wanix `qjs` task. The driver that claimed it is `QuickJsTaskDriver`: its `check` returns true for any task whose program ends in `.js`, and its `start` reads that script out of the task's namespace and evaluates it (`crates/wanix-qjs/src/driver.rs:82-102`). That is the whole registration contract — a task kind is "a driver that recognizes a command and knows how to run it." Allocate a task whose `cmd` is `main.js`, start it, and the qjs driver is the one that answers. (`.wasm` files are claimed the same way by a sibling driver; see [compiled wasm task driver](/concepts/compiled-wasm-task-driver).)

## check claims .js; start reads the script and runs it

`start` does not take a path argument. It takes the `Task`, because the script's location is a fact *about the task*, not about the call. Inside `run_task_with_runtime_limits` the driver resolves the program name from `task.cmd()`, then reads the source from `task.namespace()` — the task's private file view, not the host filesystem (`crates/wanix-qjs/src/runner.rs:196-263`). So a `qjs` task can only read what its namespace exposes. There is no `fopen("/etc/passwd")` that reaches your disk; there is only the namespace, and what the namespace binds.

From the same task fields it builds the live WASI host: stdio fds 0/1/2 come from the task's fd table, the working directory from the task's `cwd`, and `argv`/`env` from `task.cmd()` and the task's environment lines. When evaluation finishes, the driver reads the guest's requested exit code and records it with `task.set_exit(...)`; a failure short-circuits to exit `"1"` (`crates/wanix-qjs/src/runner.rs:247-262`, `crates/wanix-qjs/src/driver.rs:95-99`). After the task returns, `#task/<id>/exit` holds that status — the exit is observable as a file, like everything else.

## Module vs eval, and the console prelude

Before evaluating, the runner asks one question of the source: does it use module syntax? `uses_module_syntax` scans for a line beginning with `import ` or `export ` (`crates/wanix-qjs/src/lib.rs:170-175`). If so, the source is evaluated as an ES module under its own filename, so `import { x } from "./lib.js"` resolves through a namespace-backed module loader; otherwise it is evaluated as a plain script. Either way, a small `CONSOLE_PRELUDE` is installed first so `print(...)`, `console.log(...)`, and `console.error(...)` route to the task's stdout and stderr fds (`crates/wanix-qjs/src/lib.rs:127-133`). QuickJS has no `console` of its own; Wanix supplies one that writes to files.

The runner also defines the task globals — most visibly `scriptArgs`, a frozen array assembled from the task's argv and exposed to the guest as a real JavaScript value (`crates/wanix-qjs/src/host_api.rs:14-27`). `scriptArgs[0]` is the script name; the rest are the arguments you passed.

## What the guest sees

A qjs guest reaches everything through standard QuickJS surfaces, not a bolted-on bridge:

```js
import * as std from "qjs:std";
import * as os from "qjs:os";

// argv and env, the ordinary way
std.out.puts("args: " + scriptArgs.slice(1).join(" ") + "\n");
std.out.puts("mode: " + std.getenv("MODE") + "\n");

// the task's own identity is a file in its namespace
std.out.puts("id: " + std.loadFile("#task/self/id").trim() + "\n");

// storage is a file; no client object, just read and write
std.writeFile("#kv/greeting", "hello from a qjs task");
std.out.puts(std.loadFile("#kv/greeting") + "\n");
```

`qjs:std` and `qjs:os` are the engine's own standard library. `std.loadFile`/`std.writeFile` and `os.open`/`os.read`/`os.write` all go through the live WASI host, so they read and write the task's namespace and mirror dynamic fds back into the task's fd table. Reading `#task/self/id` works because the task's identity is a service file in its namespace; reading `#kv/greeting` works because `#kv` is a plain filesystem the namespace can resolve from the root regardless of cwd. The guidance is firm: guest JavaScript uses `qjs:std`, `qjs:os`, `scriptArgs`, stdio, env, and service files — *not* a `globalThis.Wanix` object. See [guest JS guardrails](/concepts/guest-js-guardrails).

## Under the hood

QuickJS runs as a WASI Preview 1 reactor on Wasmtime. Its pointers are offsets into WebAssembly linear memory, which is exactly why a QuickJS snapshot can be a portable VM image rather than a JavaScript serialization (`crates/wanix-qjs-engine/docs/architecture.md:19-23`; see [QuickJS snapshots are VM images](/concepts/quickjs-snapshots-are-vm-images)). Cold start is not free — compiling the ~1.7 MiB fixture costs roughly half a second — so `from_bundled_wasm` deserializes a warm, owner-private cached artifact instead, and a missing or untrusted cache only forfeits the speedup (`crates/wanix-qjs/src/lib.rs:99-118`; see [compiled artifact cache](/concepts/compiled-artifact-cache)).

After the script's top-level evaluation returns, the driver drains a *bounded* amount of event-loop work: already-due microtasks always run, future `os.setTimeout`/`setInterval` timers only fire within an explicit `event_loop_wait_budget` (default zero), and ready-IO handlers run for a fixed number of turns because QuickJS cannot tell an idle poll from one that ran a handler (`crates/wanix-qjs/src/driver.rs:33-52`, `crates/wanix-qjs/src/runner.rs:163-181`). The driver also carries two safety knobs: an interrupt-poll budget that asks QuickJS to stop CPU-bound code, and a heap byte limit that stops allocation-heavy code (`crates/wanix-qjs/src/driver.rs:54-73`). These are host policy, not a kernel — a tripped budget ends *this run* with a non-zero exit, nothing more.

## See also

- [compiled wasm task driver](/concepts/compiled-wasm-task-driver) — the sibling driver that claims `.wasm` the same way.
- [two tiers, one substrate](/concepts/two-tiers-one-substrate) — interpreted JS and compiled wasm sharing one Wasmtime and one filesystem.
- [guest JS guardrails](/concepts/guest-js-guardrails) — what guest code should and should not reach for.
- [QuickJS snapshots are VM images](/concepts/quickjs-snapshots-are-vm-images) — why a snapshot is linear memory, not serialized objects.
- [bounded execution policy](/concepts/bounded-execution-policy) — the interrupt and memory knobs in full.
- [the #task device](/devices/task) — the service files (`new`, `self/id`, `exit`) a qjs task exposes.
- Next flow step: [JS outside Chrome](/learn/js-outside-chrome).

## Status / honest limits

- The bounded-execution knobs — interrupt-poll budget, heap limit, event-loop wait budget — are host policy applied per run (`crates/wanix-qjs/src/driver.rs:54-73`). They are not a scheduler, not signals, and not a general task-cancellation model. A tripped budget ends the current run with exit `"1"`.
- A qjs task runs to completion synchronously on the calling thread (`crates/wanix-qjs/src/driver.rs:87-101`). There is no preemption between tasks; concurrency comes from running tasks on separate threads, not from time-slicing one.
- The WASI surface is command-style. Async timers fire only within an explicit wait budget that defaults to zero, and `poll_oneoff`-style readiness is not the execution model — long-lived event loops are not the shape this driver is built for.
