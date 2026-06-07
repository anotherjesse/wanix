---
title: The Shared WASI / fd Contract
slug: concepts/shared-wasi-fd-contract
pageType: concept
oneLiner: task_wasi_config builds the live WASI config — namespace, cwd preopen, argv/env, stdio fds — once for both runtimes, mirroring dynamic file fds back into the task table.
audience: [developer]
tags: [wasi, task, shipped, two-tiers]
sourceRefs:
  - crates/wanix-wasi/src/task_config.rs:20-78
  - crates/wanix-wasi/src/lib.rs:8-24
  - crates/wanix-wasi/src/ctx.rs:126-141
  - crates/wanix-wasi/src/ctx/path.rs:12-19
  - crates/wanix-wasm/src/driver.rs:7-50
  - crates/wanix-wasm/src/runner.rs:71-95
  - crates/wanix-qjs/src/task_stdio.rs:1-8
  - crates/wanix-qjs/src/lib.rs:157-161
  - docs/adrs/0002-quickjs-wasi-task-runtime.md:54-59
seeAlso:
  - concepts/wanix-backed-wasi
  - concepts/command-style-wasm-linker
  - concepts/task-drivers
  - concepts/wasmtime-as-substrate
  - concepts/two-tiers-one-substrate
  - concepts/the-fd-table
prerequisites:
  - concepts/the-fd-table
  - concepts/task-drivers
usedInFlows: []
honestLimits:
  - "The two runtimes share the WasiCtx and task_wasi_config, but not the linker: qjs threads the ctx through its own QuickJS engine host import path, while compiled wasm uses the standalone wanix-wasi-host crate's add_to_linker."
  - "Only regular-file opens are mirrored into the task fd table; directory fds stay WASI-internal until a task fd contract needs them (ADR 0002)."
  - "wanix-wasi-host is a command-style WASI subset: poll_oneoff is NOSYS, so there is no readiness wait for the compiled-wasm runner."
---

# The Shared WASI / fd Contract

`task_wasi_config` builds the live WASI config — namespace, cwd preopen, argv/env, stdio fds — once for both runtimes, mirroring dynamic file fds back into the task table.

A QuickJS script and a compiled `.wasm` program are two very different things to execute, but to Wanix they are the *same kind of task*: each gets a private namespace, a cwd, an argv, an environment, three standard fds, and a fd table that grows as the guest opens files. The temptation is to wire that up twice — once in the `qjs` driver, once in the `wasm` driver — and slowly let the two drift. Wanix refuses. There is exactly one function that turns a `Task` into a live WASI configuration, and both runtimes call it. This page is about that function and the fd contract it enforces.

## One builder for both tiers

Start a JavaScript task and a compiled wasm task against the same in-memory filesystem and watch them produce identical file state — that is what the `shared_vfs_differential` test and the `shared_vfs` example demonstrate. The reason they *can* is that neither driver assembles its own WASI world. Both call `task_wasi_config(&task)` (`crates/wanix-wasi/src/task_config.rs:20`):

```rust
pub fn task_wasi_config(task: &Task) -> WasiConfig {
    let mut config = WasiConfig::new(task.namespace())
        .with_root_preopen_source(task_wasi_cwd(task))
        .with_args(task_wasi_argv(task))
        .with_env(task_wasi_env(task))
        .with_fd_observer(TaskWasiFdMirror::new(task.clone()));
    // ...fds 0/1/2 wired below
}
```

The compiled-wasm driver imports it directly (`crates/wanix-wasm/src/driver.rs:50` — `let config = task_wasi_config(task);`), and the qjs driver re-exports the very same symbol so its call site stays stable (`crates/wanix-qjs/src/task_stdio.rs:8` — `pub(crate) use wanix_wasi::task_wasi_config;`). One source of truth, two callers. ADR 0002 makes this binding explicit: the wasm driver "builds its live config through the same `wanix-wasi` task config path as qjs," not reimplemented per runtime (`docs/adrs/0002-quickjs-wasi-task-runtime.md:54-59`).

## What the builder wires

Read the five lines above as five contracts:

- **The namespace backs WASI.** `WasiConfig::new(task.namespace())` makes the guest's filesystem syscalls resolve through the task's per-process namespace — not the host OS. A guest `open` is a namespace resolution.
- **cwd is the root preopen.** `with_root_preopen_source(task_wasi_cwd(task))` replaces fd 3's source path with the task's cwd while keeping its guest name as `.` (`crates/wanix-wasi/src/config.rs:236`). The guest sees a standard preopened root directory; behind it is wherever the task is rooted.
- **argv/env come from the task command.** `task_wasi_argv` / `task_wasi_env` pull the WASI process arguments and `KEY=value` environment from the shared `wanix_task` command extraction, so the guest's `args_get`/`environ_get` reflect how the task was launched.
- **stdio fds 0/1/2 are the task's fds.** The builder checks `task.fd_numbers()` and, for each of `STDIN`/`STDOUT`/`STDERR` the task actually has open, attaches a `TaskFdFile` that reads/writes straight through the live task fd-table entry. An unopened standard fd stays closed in the guest — there is no fabricated empty stdin.

That is the whole surface a task hands to a WASI runtime. Nothing runtime-specific lives in it.

## fds the guest opens flow back into the task

The interesting half is dynamic. When a guest calls `path_open` and gets a fresh fd, that fd should be *observable* — `#task` inspection and snapshot policy both depend on knowing what a task has open. The builder attaches a `TaskWasiFdMirror` as the config's fd observer, and the `WasiCtx` calls it on every regular-file open and close (`crates/wanix-wasi/src/task_config.rs:60-78`):

```rust
fn file_opened(&self, fd: WasiFd, file: WasiFile, path: &NormalizedPath) -> Result<(), Errno> {
    let task_fd = Fd::new(fd.get());
    self.task.insert_fd_if_vacant(task_fd, Box::new(file), path.clone())?
}
fn fd_closed(&self, fd: WasiFd) -> Result<(), Errno> {
    // close_fd, tolerating an already-absent fd
}
```

So a guest that opens `data.txt` at WASI fd 4 makes `task.fd_numbers()` report `[4]`, and the host can `read_fd(4, ...)` and see the same bytes the guest sees. Close the guest fd — or drop the `WasiCtx` entirely — and the mirrored task fd is released. The mirror also numbers around collisions: `file_fd_available` skips any number the task already holds, so an existing fd 4 keeps its identity and the dynamic open lands at fd 5. Two important boundaries: only **regular-file** opens reach the observer (directory fds stay WASI-internal until a task fd contract needs them, per ADR 0002), and the fd *number* the guest sees and the task fd *number* are deliberately the same `u32`, so there is no translation table to keep in sync.

## A guest reaches any `#name` device from the root

There is one resolution rule that makes service devices usable from inside a guest no matter where its cwd is. Normally a relative WASI path is joined to the preopen base. But if the first path component is a known service device, the path resolves from the namespace root instead (`crates/wanix-wasi/src/ctx.rs:137`, `crates/wanix-wasi/src/ctx/path.rs:12-19`):

```rust
const ROOTED_SERVICE_DEVICES: &[&str] = &[
    "#task", "#term", "#kv", "#pipe", "#plumb", "#cas", "#agent", "#mesh", "#cpu",
];
```

So a `qjs` or `wasm` task buried three directories deep can still `open("#kv/http-counter")` and hit the key/value device, because `#kv` is recognized and rooted rather than joined to the cwd. An *unrecognized* `#name` (a file literally named `#foo` in the cwd) stays cwd-relative — the rule is a closed list of Plan 9 service names, not a blanket "any `#` escapes the root." This is the same `#`-device convention the [service devices](/concepts/service-devices) page describes, enforced at the WASI syscall boundary.

## What the two runtimes share — and what they do not

They share the `WasiConfig` builder and the `WasiCtx` it produces: the namespace backing, preopen, argv/env, stdio, the fd mirror, and the service-path rule are all identical. That is the Wanix-owned half of the WASI boundary, and ADR 0002 reserves it for Wanix: task identity, live WASI semantics, fd/service state.

They do **not** share the linker — the engine half. The compiled-wasm runner builds a Wasmtime `Linker` and calls `wanix_wasi_host::add_to_linker(&mut linker)` from the standalone [command-style wasm linker](/concepts/command-style-wasm-linker) crate (`crates/wanix-wasm/src/runner.rs:78`). QuickJS does not link Wasmtime imports for the guest; it wraps the same `WasiCtx` inside its own engine-local QuickJS host (`WanixQuickJsWasiHost`, `crates/wanix-qjs/src/lib.rs:160-161`) so `qjs:std`/`qjs:os` calls reach Wanix-backed WASI. Same ctx, two import paths — which is exactly the split ADR 0002 intends: Wanix owns the semantics, each runtime crate owns its engine mechanics. See [two tiers, one substrate](/concepts/two-tiers-one-substrate).

## See also

- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — why WASI syscalls resolve through namespaces, not the host OS.
- [The command-style wasm linker](/concepts/command-style-wasm-linker) — `wanix-wasi-host`, the engine half the compiled-wasm tier links.
- [Task drivers](/concepts/task-drivers) — how `check`/`start` dispatch a `.js` or `.wasm` cmd to a runtime.
- [The fd table](/concepts/the-fd-table) — the task-owned fds that stdio and dynamic opens mirror into.
- [Two tiers, one substrate](/concepts/two-tiers-one-substrate) — interpreted and compiled WASI tasks over one Wasmtime.
- [Wasmtime as substrate](/concepts/wasmtime-as-substrate) — the execution engine under both tiers.

## Status / honest limits

- **Shared ctx, separate linkers.** The two runtimes share `task_wasi_config` and the `WasiCtx`, but not the linker. The compiled-wasm tier uses `wanix-wasi-host`'s `add_to_linker`; qjs threads the ctx through its own QuickJS engine host import path (`crates/wanix-qjs/src/lib.rs:160-161`). They converge on semantics, not on engine plumbing.
- **Only regular files are mirrored.** Directory fds stay WASI-internal; only regular-file opens reach `TaskWasiFdMirror` and become task fds (`crates/wanix-wasi/src/task_config.rs:60-78`, ADR 0002).
- **Command-style WASI only.** `wanix-wasi-host` is a command-style subset: `poll_oneoff` is `NOSYS`, so the compiled-wasm runner has no readiness wait, and a guest runs `_start` to completion or `proc_exit` (`crates/wanix-wasm/src/runner.rs:71-95`).
- **Service paths are a fixed list.** Only the names in `ROOTED_SERVICE_DEVICES` resolve from the root; any other `#`-prefixed name stays cwd-relative (`crates/wanix-wasi/src/ctx/path.rs:12-19`).
