---
title: Compiled wasm Task Driver
slug: concepts/compiled-wasm-task-driver
pageType: concept
oneLiner: Any wasm32-wasi command module (Rust, C, Zig, Go) runs as a Wanix .wasm task at near-native speed, sharing the same sandbox and VFS as qjs.
audience: [newcomer, developer]
tags: [task-runtime, wasm, shipped, cli, caveat]
sourceRefs:
  - crates/wanix-wasm/src/driver.rs:29-68
  - crates/wanix-wasm/src/runner.rs:51-95
  - crates/wanix-wasm/src/lib.rs:1-30
  - crates/wanix-wasm/src/cache.rs:1-38
  - crates/wanix-wasm/src/task_stdio.rs:1-8
  - crates/wanix-cli/src/wasm.rs:1-48
  - crates/wanix-cli/tests/shared_vfs_differential.rs:86-125
seeAlso:
  - concepts/qjs-task
  - concepts/two-tiers-one-substrate
  - concepts/command-style-wasm-linker
  - concepts/compiled-artifact-cache
  - devices/task
prerequisites:
  - concepts/task-drivers
  - concepts/shared-wasi-fd-contract
usedInFlows: []
honestLimits:
  - "Command-style WASI only: poll_oneoff is registered as ENOSYS, so there is no readiness or async I/O — a guest that blocks on poll gets NOSYS, not a wakeup."
  - "A .wasm task is an exec device task; exec devices (#task/#agent/#cpu) are local-trust only and not exposed to untrusted peers. There are no hard CPU or memory limits yet."
  - "The module cache directory is a trust boundary: a non-owner-private dir is ignored (you forfeit the warm-start speedup), not trusted."
---

# Compiled wasm Task Driver

Any `wasm32-wasi` command module (Rust, C, Zig, Go) runs as a Wanix `.wasm` task at near-native speed, sharing the same sandbox and VFS as `qjs`.

A QuickJS script is one way to put a task on the substrate; a compiled `.wasm` module is the other. Both are WASI guests, both run on Wasmtime, and both reach the file world through the same `WasiConfig` built from the task's namespace. The interpreter tier is convenient; the compiled tier is fast. The point of this page is that switching tiers does not change the contract — only the speed and the source language. That symmetry is the whole reason `wanix-wasm` exists as a *second* task driver instead of a one-off runner.

## Show it: run a compiled guest

The first build step in any flow brings the binary up:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'
```

Then run a compiled module the same way you run a script:

```sh
wanix-rust wasm ./guest.wasm /in.txt /out.txt
```

`guest.wasm` reads `/in.txt`, writes `/out.txt`, and exits — no host glue. The CLI preopens the working directory as the guest's namespace root, builds argv as `[program, args...]`, captures stdout/stderr, and returns the guest's exit code (`crates/wanix-cli/src/wasm.rs:30-48`). This is the standalone one-shot runner: it compiles and runs a `.wasm` file directly and does *not* go through the task model (`crates/wanix-cli/src/wasm.rs:9-15`). The task-driver integration is the other half, below.

## `.wasm` is a first-class task kind

Inside a served namespace, a `.wasm` program is a task the same way `qjs` and `noop` are. The piece that makes that true is `WasmTaskDriver`, and it earns the job with two methods (`crates/wanix-wasm/src/driver.rs:29-43`):

```rust
impl TaskDriver for WasmTaskDriver {
    fn check(&self, task: &Task) -> bool {
        task_program_for_check(task).is_some_and(|program| program.ends_with(".wasm"))
    }

    fn start(&self, task: &Task) -> FsResult<()> { /* compile, run, record exit */ }
}
```

`check` claims any task whose program name ends in `.wasm`. When a task is allocated as `auto` and started, the task table walks its registered drivers, asks each one `check`, and the wasm driver raises its hand for a `.wasm` command — so `#task/new/wasm` (or an `auto` task with a `.wasm` cmd) routes here without the caller naming the driver. This is the Plan 9 idea that the *kind* of a task is data, not a hardcoded branch: see [task drivers](/concepts/task-drivers).

`start` runs the program and records the result. On a normal return it calls `task.set_exit` with the guest's exit code; on an error it sets exit `1` and propagates the error (`crates/wanix-wasm/src/driver.rs:34-43`). The exit is observable on the task afterward, which is what `#task/<id>` reads back.

## What `start` actually does

`start` defers to `run_wasm_task`, four steps with nothing hidden (`crates/wanix-wasm/src/driver.rs:45-54`):

1. `task_command(task)` resolves the program path and argv from the task.
2. `read_namespace_bytes(&task.namespace(), &command.program)` reads the module bytes **from the task's own namespace** — 64 KiB at a time. The module is not a host file path; it is whatever the task can open at that path in its file view. A `.wasm` that lives behind a bind, or even imported across the mesh, is opened the same way.
3. `WasiRunner::from_bytes_cached(&bytes, &module_cache_dir())` compiles (or loads a cached compile of) the module.
4. `task_wasi_config(task)` builds the live WASI config, and `runner.run(config)` executes `_start`.

The runner's `run` is where Wasmtime meets WASI (`crates/wanix-wasm/src/runner.rs:71-95`). It builds a `WasiCtx` from the config, makes a `Store`, creates a `Linker`, and registers the imports with one call: `wanix_wasi_host::add_to_linker(&mut linker)`. That linker is the standalone, engine-free WASI Preview 1 host — the same one the bare `WasiRunner` uses, with no task or engine coupling. See [the command-style wasm linker](/concepts/command-style-wasm-linker).

Then it instantiates, looks up `_start` (erroring clearly if the module has none — "not a WASI command"), and calls it. The exit handling is the careful part: a clean `proc_exit` unwinds the guest as a Wasmtime trap, so `run` checks `store.data().exit_code()` first and *recovers* that code rather than reporting a trap (`crates/wanix-wasm/src/runner.rs:87-94`). A normal return with no recorded code is exit `0`; a real trap with no code is the error.

## One config path, one fd contract

The most load-bearing detail is what `task_wasi_config` is. It is not a wasm-specific function — it is re-exported straight from `wanix-wasi` (`crates/wanix-wasm/src/task_stdio.rs:1-8`). The namespace, stdio, argv, env wiring, and the dynamic fd mirroring all live in `wanix-wasi`, so *every* WASI task runtime — QuickJS and the compiled-wasm driver — follows one fd-mirroring contract. A `qjs` task and a `.wasm` task built from the same `Namespace` share one filesystem: a write by one is visible to the other, through the same `WasiCtx`. That is the [shared WASI fd contract](/concepts/shared-wasi-fd-contract), and it is what makes [two tiers, one substrate](/concepts/two-tiers-one-substrate) more than a slogan.

The differential test proves it without hand-waving. `qjs_and_rust_wasm_see_identical_directory_listing` runs a QuickJS program (`os.readdir` + `os.lstat`) and a compiled Rust guest (`std::fs::read_dir`) against the *same* `MemFs` namespace, and asserts both produce a byte-for-byte identical sorted `name type` listing (`crates/wanix-cli/tests/shared_vfs_differential.rs:86-125`). Two engines, one VFS, one answer — because both route through `WasiCtx::fd_read_dir` on the same backend. Sibling tests in the same file have qjs write a file and rust-wasm rename it (or truncate it, or read its symlink) and watch the change appear on the other side.

## Under the hood: a distinct cache subdir

Compilation is the cost the compiled tier pays up front. Cranelift-compiling a non-trivial guest is tens to hundreds of milliseconds; `from_bytes_cached` stores the Wasmtime serialized module keyed by `sha256(bytes)` and deserializes it in well under a millisecond on a warm run (`crates/wanix-wasm/src/runner.rs:51-60`). The cache is advisory: a missing, stale, or *untrusted* artifact falls back to a fresh compile.

The wasm runner gets its *own* cache subdirectory, `wasm-module-cache`, distinct from the bundled-qjs `qjs-module-cache`, so the two runtimes never collide (`crates/wanix-wasm/src/cache.rs:1-38`). The directory is owner-private by resolution — `WANIX_WASM_CACHE_DIR` if set and trusted, else a per-user cache home, else a UID-scoped temp subdir. Because a cached artifact is loaded through `unsafe Module::deserialize`, the cache layer verifies leaf-directory and artifact ownership before reading anything; a world-writable or symlinked path is refused, and the runner just recompiles. The cache is the subject of its own page, [the compiled-artifact cache](/concepts/compiled-artifact-cache).

## See also

- [The qjs task](/concepts/qjs-task) — the interpreter sibling that shares this exact config path.
- [Two tiers, one substrate](/concepts/two-tiers-one-substrate) — why interpreted and compiled tasks run on one Wasmtime substrate.
- [The command-style wasm linker](/concepts/command-style-wasm-linker) — the engine-free WASI host this driver links against.
- [The compiled-artifact cache](/concepts/compiled-artifact-cache) — the trust-boundary-verified warm-start cache.
- [Task drivers](/concepts/task-drivers) — how `check`/`start` registration selects this driver by `.wasm`.
- [The #task device](/devices/task) — the files that allocate, start, and observe a `.wasm` task.

## Status / honest limits

These are engineering boundaries, stated once.

- **Command-style WASI only.** The shared linker registers `poll_oneoff` as `ERRNO_NOSYS` (`crates/wanix-wasm/src/lib.rs:14-17`). The driver supports command-style guests — `_start`, fd/path I/O, args/env, clock, exit — and is **not** a general-purpose WASI host. A guest that blocks on `poll` for readiness gets NOSYS, not a wakeup. There is no async I/O.
- **A `.wasm` task is an exec device task, and exec is local-trust only.** `#task`, like `#agent` and `#cpu`, runs caller-supplied compute and is not exposed to untrusted or public peers. The isolation here is cheap and scalable, not a claim of safety for arbitrary untrusted code, and there are no hard CPU or memory limits on a running guest yet.
- **The cache is a trust boundary, not just a speedup.** A non-owner-private cache directory is ignored, not trusted: you forfeit the warm-start deserialize and the runner recompiles cleanly (`crates/wanix-wasm/src/runner.rs:51-60`).
