---
title: Command-Style wasm Linker (poll_oneoff = NOSYS)
slug: concepts/command-style-wasm-linker
pageType: concept
oneLiner: wanix-wasi-host is a generic WASI Preview 1 linker for command-style wasm guests; poll_oneoff returns ERRNO_NOSYS, so there is no readiness polling, and qjs keeps its own richer host path.
audience: [developer]
tags: [wasm, wasi, runtime, shipped, caveat]
sourceRefs:
  - crates/wanix-wasi-host/src/lib.rs:1-58
  - crates/wanix-wasi-host/src/core.rs:34-39
  - crates/wanix-wasi-host/src/process.rs:7-18
  - crates/wanix-wasi-host/src/fd.rs:16-198
  - crates/wanix-wasi-host/src/vectors.rs:8-25
seeAlso:
  - concepts/compiled-wasm-task-driver
  - concepts/wanix-backed-wasi
  - concepts/shared-wasi-fd-contract
  - concepts/qjs-task
  - concepts/two-tiers-one-substrate
prerequisites:
  - concepts/compiled-wasm-task-driver
usedInFlows: []
honestLimits:
  - poll_oneoff returns ERRNO_NOSYS, so there is no readiness polling — this is a command-style subset, not a server-style WASI host.
  - Wanix runs two host-import paths, not one — the wasm runner uses this linker, qjs uses its own engine-local path; only the WasiCtx and task-config contracts are shared.
  - random_get is a deterministic fill and clock_time_get reports the host-supplied clock — this runner targets reproducible tasks, not entropy-grade randomness.
---

# Command-Style wasm Linker (poll_oneoff = NOSYS)

`wanix-wasi-host` is a generic WASI Preview 1 linker for command-style wasm guests; `poll_oneoff` returns `ERRNO_NOSYS`, so there is no readiness polling, and qjs keeps its own richer host path.

When you run `wanix wasm hello.wasm`, a compiled `wasm32-wasi` module starts, reads and writes files and fds, sees its argv and environment, asks the clock for the time, and exits with a status Wanix records. That whole surface — and nothing more — is what one small crate registers on the Wasmtime linker. It is deliberately a *command* host: a guest runs `_start`, does its I/O, and exits. It does not get to block on readiness, because the one syscall that would let it do that is wired to "not implemented." This page explains why that boundary exists, what is on either side of it, and why Wanix runs two host paths instead of forcing every guest through one.

## What "command-style" means

A WASI guest can be written two ways. A *command* runs to completion: it starts, processes its inputs, and exits — like a Unix program invoked from a shell. A *reactor* stays resident and reacts to events over time, which means it needs to wait for I/O to become ready. `wanix-wasi-host` is a command host. The doc comment on the crate says so plainly: it is "the generic command-style WASI Preview 1 Wasmtime linker over `WasiCtx`" (`crates/wanix-wasi-host/src/lib.rs:1`). The `wanix-wasm` runner uses it directly; the QuickJS engine does not.

## The registered subset

`add_to_linker` registers exactly five families of imports and no more (`crates/wanix-wasi-host/src/lib.rs:51-58`):

- **Process lifecycle** — `proc_exit`.
- **Core, non-filesystem syscalls** — `sched_yield`, `random_get`, `clock_time_get`, and `poll_oneoff`.
- **Argv and environment** — `args_sizes_get`, `args_get`, `environ_sizes_get`, `environ_get` (`crates/wanix-wasi-host/src/vectors.rs:8`).
- **File-descriptor I/O** — `fd_read`, `fd_write`, `fd_close`, `fd_seek`, `fd_tell`, the `fd_fdstat_*` and `fd_prestat_*` pair, `fd_filestat_*`, and `fd_readdir` (`crates/wanix-wasi-host/src/fd.rs:16`).
- **Path operations** — `path_open`, `path_filestat_get`, `path_readlink`, `path_create_directory`, `path_remove_directory`, `path_unlink_file`, `path_rename`, `path_symlink`.

That list is the contract a command guest can rely on. Every fd and path call delegates straight into the guest's `WasiCtx` — the live, Wanix-namespace-backed WASI context — so opening a path or reading an fd is a real operation against the task's namespace, not a sandbox shim. See [Wanix-backed WASI](/concepts/wanix-backed-wasi).

## Why poll_oneoff returns ERRNO_NOSYS

The one conspicuous gap is readiness. `poll_oneoff` is the WASI call a reactor uses to wait until an fd is ready to read or write. Here it is one line (`crates/wanix-wasi-host/src/core.rs:34-39`):

```rust
linker.func_wrap(
    m,
    "poll_oneoff",
    |_: Caller<'_, S>, _i: i32, _o: i32, _n: i32, _ne: i32| ERRNO_NOSYS,
)?;
```

`ERRNO_NOSYS` is errno 52 — "function not implemented" (`crates/wanix-wasi-host/src/lib.rs:43`). A guest that calls `poll_oneoff` gets a clean, honest error rather than a fake "ready" answer or a hang. That is the command-style boundary made explicit: a command does its I/O inline and exits, so it never needs to block on an event. Two neighbouring core calls round out the deterministic posture: `random_get` fills the buffer with a fixed byte because "this runner targets reproducible tasks," and `clock_time_get` returns whatever nanosecond value the host supplies through `clock_time_ns()` (`crates/wanix-wasi-host/src/core.rs:13-33`).

## Generic over a WasiHost backing

Nothing in this crate knows about engines or tasks. It is generic over a `WasiHost` trait that a Wasmtime store-state implements (`crates/wanix-wasi-host/src/lib.rs:29-38`):

```rust
pub trait WasiHost: Send {
    fn wasi(&mut self) -> &mut WasiCtx;     // the namespace-backed context
    fn clock_time_ns(&self) -> u64;         // the clock reported to the guest
    fn on_proc_exit(&mut self, code: i32);  // the exit-code hook
}
```

A consumer supplies its `WasiCtx`, a clock, and an exit hook; everything that differs per consumer stays in the consumer. As the crate puts it, "fixing a Preview 1 detail here fixes it for every consumer of this linker" (`crates/wanix-wasi-host/src/lib.rs:11-13`). Today the consumer is `wanix-wasm`'s task driver — see [the compiled wasm task driver](/concepts/compiled-wasm-task-driver).

## Under the hood: proc_exit is a trap

A guest exits by calling `proc_exit(code)`. There is no return-from-`_start` path to thread a status through, so the linker records the code and then unwinds the guest with a trap (`crates/wanix-wasi-host/src/process.rs:7-18`):

```rust
caller.data_mut().on_proc_exit(code);
// Unwind the guest; the consumer recovers the code from its exit hook.
Err::<(), _>(Error::msg(format!("proc_exit({code})")))
```

The error stops execution; the consumer reads the recorded code from its `on_proc_exit` hook and reports it as the task's exit status. The trap is the mechanism, not a failure — the exit code is captured before the unwind.

## See also

- [Compiled wasm task driver](/concepts/compiled-wasm-task-driver) — the `wanix-wasm` driver that builds a `WasiHost` and runs `_start`.
- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — the live `WasiCtx` every fd and path call resolves through.
- [Shared WASI fd contract](/concepts/shared-wasi-fd-contract) — how fds 0/1/2 mirror into the namespace the same way for both runtimes.
- [The qjs task](/concepts/qjs-task) — the other guest kind, which uses its own engine-local host path.
- [Two tiers, one substrate](/concepts/two-tiers-one-substrate) — compiled wasm and interpreted JS over one Wasmtime.

## Status / honest limits

- **`poll_oneoff` is `ERRNO_NOSYS`.** This is a command-style WASI subset, not a server-style host. A reactor that needs readiness polling will get errno 52 (`crates/wanix-wasi-host/src/core.rs:34-39`).
- **Two host paths, not one.** Only the `wanix-wasm` runner uses this linker. The QuickJS engine keeps its own engine-local host-import path with snapshot blockers, live fd readiness, restore reattachment, and a richer `poll_oneoff`; the two share only the `wanix-wasi` `WasiCtx` and task-config contracts, not this linker (`crates/wanix-wasi-host/src/lib.rs:5-13`).
- **Determinism over fidelity.** `random_get` fills a fixed byte and `clock_time_get` reports the host-supplied clock — appropriate for reproducible tasks, not for entropy-grade randomness or wall-clock precision (`crates/wanix-wasi-host/src/core.rs:13-33`).
