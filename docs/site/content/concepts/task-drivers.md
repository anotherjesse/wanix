---
title: Task Drivers
slug: concepts/task-drivers
pageType: concept
oneLiner: A driver's check() auto-selects a runtime by program suffix (.js->qjs, .wasm->wasm) and start() runs it; runtimes register from above so the task core stays engine-free.
audience: [developer]
tags: [tasks, drivers, runtime, shipped, cli]
sourceRefs:
  - crates/wanix-task/src/driver.rs:1-28
  - crates/wanix-task/src/table.rs:47-154
  - crates/wanix-qjs/src/driver.rs:82-102
  - crates/wanix-wasm/src/driver.rs:29-68
  - crates/wanix-cli/src/serve/roots.rs:189-191
  - crates/wanix-cli/src/serve/discovery.rs:165
  - docs/adrs/0002-quickjs-wasi-task-runtime.md:31-45
seeAlso:
  - concepts/tasks-own-process-identity
  - concepts/shared-wasi-fd-contract
  - concepts/qjs-task
  - concepts/compiled-wasm-task-driver
  - reference/crate-map-and-layering
prerequisites:
  - concepts/tasks-own-process-identity
usedInFlows:
  - {flow: add-driver-or-transport, step: 1}
honestLimits:
  - The registered drivers today are noop, qjs (.js), and wasm (.wasm); there is no third runtime.
  - Auto-selection is by program suffix only — a .js file selects qjs even if it is not valid JavaScript.
  - A kind with no registered driver fails start() with NotFound; the orchestration layer must register every kind it allocates.
---

# Task Drivers

A driver's `check()` auto-selects a runtime by program suffix (`.js` -> qjs, `.wasm` -> wasm) and `start()` runs it; runtimes register from above so the task core stays engine-free.

You allocated a task and gave it a command line. Now something has to *run* it — turn `main.js` into a QuickJS interpreter session, or `tool.wasm` into a Wasmtime instance. A **task driver** is that something. It is a two-method trait the task core knows about but never implements: the runtime crates implement it and hand it to the table from above. That direction matters, because it is the only thing keeping `wanix-task` free of Wasmtime, QuickJS, and every other engine.

## Show: a command picks its own runtime

Run a `.wasm` program as a Wanix task:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

wanix wasm fixtures/rust-guest.wasm
```

You did not tell Wanix "this is a wasm task." You handed it a program ending in `.wasm`, and the task table found the driver that claimed it. The same is true the other way: `wanix qjs main.js` runs the QuickJS driver because the program ends in `.js`. The selection lives in two lines of code, one per runtime:

```rust
// crates/wanix-wasm/src/driver.rs:30
fn check(&self, task: &Task) -> bool {
    task_program_for_check(task).is_some_and(|p| p.ends_with(".wasm"))
}

// crates/wanix-qjs/src/driver.rs:83
fn check(&self, task: &Task) -> bool {
    task_program_for_check(task).is_some_and(|p| p.ends_with(".js"))
}
```

That auto-selection is the **driver** doing its first job: looking at a task and answering "is this mine to start?"

## check() claims, start() launches

The whole contract is two methods (`crates/wanix-task/src/driver.rs:6-18`):

```rust
pub trait TaskDriver: Send + Sync {
    fn check(&self, _task: &Task) -> bool { false }   // can I auto-start this?
    fn start(&self, task: &Task) -> FsResult<()>;     // run it.
}
```

`check()` defaults to `false`, so a driver that only runs explicitly named kinds (the `NoopDriver` used by tests and early allocation flows) inherits "never auto-claim" for free. `start()` is the only required method — it reads the task's program, environment, cwd, argv, and fds, builds a live runtime, runs the guest, and records the exit. The wasm driver's `start()` is the canonical shape (`crates/wanix-wasm/src/driver.rs:34-54`): read the module bytes from the task's *own* namespace, compile them through the shared module cache, build a `WasiConfig` from the task, run `_start`, and write the guest exit code back through `Task::set_exit`. On any failure it still records `"1"` so the exit status stays observable. The qjs driver does the same with QuickJS bounded-execution limits in place of a raw `_start`.

## Why the task core must not depend on the engines

`wanix-task` defines `TaskDriver` and the registry; it does *not* know what qjs or wasm are. If it did, the dependency arrows would point upward — the process model would import the execution substrate — and you could never add a runtime without editing the core. The dependency rule (`AGENTS.md`, and ADR 0002 at lines 31-45) is blunt: `wanix-task` must not depend on `wanix-wasi`, `wanix-qjs`, or `wanix-wasm`. Drivers register *from* the orchestration layer, downward into the core.

You can see the registration site in `serve` (`crates/wanix-cli/src/serve/roots.rs:189-191`):

```rust
table.register_noop_driver("noop")?;
table.register_driver("qjs", Arc::new(QuickJsTaskDriver::new(quickjs_runner()?)))?;
table.register_driver("wasm", Arc::new(WasmTaskDriver::new()))?;
```

Each CLI surface that runs tasks does this same wiring (`agent_program.rs`, `qjs_term`, the `qjs`/`wasm` subcommands). The runtime crate owns its engine mechanics; the orchestration layer decides which engines exist in this process and plugs them in. The task core stays a pure, engine-free process model — the boundary ADR 0002 calls the split between Wanix-owned task state and the runtime crate's engine mechanics.

## Worked example: a new driver, reusing the shared config

Adding a runtime is implementing one trait. Suppose you want a `lua` kind for `.lua` programs:

```rust
use std::sync::Arc;
use wanix_fs::FsResult;
use wanix_task::{Task, TaskDriver, TaskTable, task_program_for_check};

struct LuaDriver;

impl TaskDriver for LuaDriver {
    fn check(&self, task: &Task) -> bool {
        task_program_for_check(task).is_some_and(|p| p.ends_with(".lua"))
    }
    fn start(&self, task: &Task) -> FsResult<()> {
        // read task.namespace() / cmd / env, run, then:
        task.set_exit("0")
    }
}

// from the orchestration layer, downward:
table.register_driver("lua", Arc::new(LuaDriver))?;
```

`task_program_for_check` (`crates/wanix-task/src/task_command.rs:108`) gives you the same program string both built-in drivers match against, so suffix selection stays consistent. If your runtime is a WASI guest, do *not* reinvent fd handling: the wasm driver builds its config through `task_wasi_config(task)` (`crates/wanix-wasi/src/task_config.rs`), the shared WASI path that wires fds 0/1/2, the cwd preopen, env, and argv from the task — the same path qjs uses. Reusing it is how a new runtime inherits the [shared WASI fd contract](/concepts/shared-wasi-fd-contract) instead of forking it.

## Under the hood: registry and dispatch

The registry is a `BTreeMap<String, Arc<dyn TaskDriver>>` on the task table (`crates/wanix-task/src/table.rs:20`). When you `start(id)` a task, the table reads the task's kind and dispatches (`crates/wanix-task/src/table.rs:122-154`):

- **A concrete kind** (`"qjs"`, `"wasm"`) looks up that exact driver and calls `start()`; an unregistered kind returns `FsError::NotFound`.
- **The `"auto"` kind** iterates the drivers, calls `check()` on each, sets the task's kind to the first match, and starts it. No match returns `Ok(())` — an allocated-but-unrunnable task, not an error. This is the dispatch behind `wanix wasm FILE.wasm`: a `.wasm` cmd allocated as `auto` resolves to the wasm driver by suffix.

`driver_kinds()` (`crates/wanix-task/src/table.rs:68`) returns the registered kinds with `"auto"` prepended — and `serve`'s discovery document derives its advertised driver list straight from that registry (`crates/wanix-cli/src/serve/discovery.rs:165`), so a client always sees exactly the kinds this process can start. The list is data, not a hardcoded string; the earlier driver-list drift is gone.

## See also

- [Tasks own process identity](/concepts/tasks-own-process-identity) — what a task *is* before a driver runs it.
- [The shared WASI fd contract](/concepts/shared-wasi-fd-contract) — the `task_wasi_config` path both WASI runtimes reuse.
- [The qjs task](/concepts/qjs-task) — the QuickJS driver in full.
- [The compiled wasm task driver](/concepts/compiled-wasm-task-driver) — the `.wasm` runtime in full.
- [Crate map and layering](/reference/crate-map-and-layering) — why the dependency arrows point downward.
- Flow: [Add a driver or transport](/learn/add-driver-or-transport) — build the `lua`-shaped driver end to end.

## Status / honest limits

- **Three kinds exist today**: `noop`, `qjs` (`.js`), and `wasm` (`.wasm`). There is no third *runtime*; the `lua` driver above is illustrative, not shipped.
- **Selection is by suffix, not by content**: `check()` matches the program string's tail (`crates/wanix-qjs/src/driver.rs:84`, `crates/wanix-wasm/src/driver.rs:31`). A `.js` file selects qjs even if its bytes are not valid JavaScript; the failure surfaces at `start()` as a guest error and exit `"1"`, not at selection.
- **A kind with no driver is an error**: `start()` on an unregistered concrete kind returns `FsError::NotFound` (`crates/wanix-task/src/table.rs:152`). The orchestration layer is responsible for registering every kind it allocates; the core will not invent one.
- **Tasks are local-trust exec.** `#task` (and the runtimes drivers launch) is a local-trust exec plane, not exposed to untrusted or public peers, and there are no hard CPU or memory ceilings yet beyond qjs's bounded-execution knobs. Treat drivers as cheap, composable isolation — not as a sandbox for arbitrary hostile code.
