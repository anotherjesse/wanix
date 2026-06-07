---
title: Wasmtime as the Execution Substrate
slug: concepts/wasmtime-as-substrate
pageType: concept
oneLiner: Wasmtime hosts guest runtimes but inherits no host filesystem or process semantics by default; Wanix supplies live WASI backed by its own namespaces.
audience: [developer, visionary]
tags: [runtime, substrate, wasi, trust-boundary, shipped, caveat, local-trust-only]
sourceRefs:
  - docs/adrs/0001-rust-native-wasmtime-runtime.md:25-43
  - docs/adrs/0002-quickjs-wasi-task-runtime.md:47-52
  - docs/rust-vs-go-wanix.md:227-283
  - docs/scaling-eli5.md:89-95
  - crates/wanix-module-cache/src/lib.rs:1-49
seeAlso:
  - concepts/wanix-backed-wasi
  - concepts/host-not-ambient-authority
  - concepts/two-tiers-one-substrate
  - concepts/rooms-not-houses
  - concepts/safe-for-untrusted-not-claimable
  - concepts/compiled-artifact-cache
prerequisites:
  - concepts/everything-is-a-file
usedInFlows: []
honestLimits:
  - Per-room (per-instance) memory isolation is real, but hard CPU/memory limits (Wasmtime epoch/fuel preemption, linear-memory caps) are NOT wired up yet.
  - Exec tiers running on this substrate (#task, #cpu, #agent) are local-trust only; "cheap, scalable isolation" is the honest claim, not "safe for arbitrary untrusted code."
canonicalCaveatFor: []
---

# Wasmtime as the Execution Substrate

Wasmtime hosts guest runtimes but inherits no host filesystem or process semantics by default; Wanix supplies live WASI backed by its own namespaces.

Wanix runs guest code — a QuickJS script, a compiled `.wasm` program — inside Wasmtime. But Wasmtime is the *substrate*, not the foundation that defines what a file, a process, or a capability means. Those semantics belong to Wanix. This page explains the inversion that makes that work: WASI, which in most systems is a door *out* to the host, becomes in Wanix an adapter *in* to Wanix-owned namespaces. The host's filesystem and process model never leak in unless you bind them on purpose.

## Substrate, not foundation

Start with the rule, because it shapes everything else. ADR 0001 states it plainly: "Wasmtime hosts guest runtimes. It is not allowed to inherit host filesystem or process semantics by default" (`docs/adrs/0001-rust-native-wasmtime-runtime.md:32-33`). Wanix owns "filesystem behavior, namespace binding and resolution, task identity, task service files, fd tables, terminal devices, and externally visible process state" (`docs/adrs/0001-rust-native-wasmtime-runtime.md:29-31`). Wasmtime supplies one thing: a sandboxed CPU and linear memory to execute WebAssembly in.

That division is deliberate. A WebAssembly engine ships with WASI Preview 1, and the default WASI implementation hands a guest the host's directories, environment, and clocks. Wanix refuses that default. A guest that asks to open a file does not reach a host path; it reaches a path in *its task's namespace*, resolved by Wanix's VFS. "Host files enter Wanix only through explicit rooted mounts with escape checks; ambient host paths are not part of the default namespace" (`docs/adrs/0001-rust-native-wasmtime-runtime.md:36-37`).

## Two guests, neither a second process model

Wanix runs two WASI guests today, and the architectural discipline is the same for both: the engine is an *engine inside a task*, never the task model.

The QuickJS runtime (`wanix-qjs-engine`) hosts the QuickJS WebAssembly reactor under Wasmtime; `wanix-qjs` adapts it into Wanix task semantics. The compiled-`wasm32-wasi` runtime (`wanix-wasm`) does the same for arbitrary command modules. In both cases Wanix — not the engine — owns task identity, the task table, cwd, env, argv, stdio, the fd table, the namespace, the exit status, and the `#task` service files (`docs/rust-vs-go-wanix.md:262-273`).

The payoff of that separation is portability: "the architecture is not 'a QuickJS app with a file API.' It is Wanix with a QuickJS task driver" (`docs/rust-vs-go-wanix.md:280-283`). Because QuickJS does not define the process model, a second driver — the compiled-wasm tier — slots in without a second notion of what a task is. Two tiers, one substrate (see [two tiers, one substrate](/concepts/two-tiers-one-substrate)).

## WASI inverted: an adapter into Wanix, not a delegation to the host

This is the conceptual hinge of the page. As `docs/rust-vs-go-wanix.md:231-233` puts it: "In many systems, WASI is a way to delegate filesystem and fd behavior to the host. Rust Wanix deliberately goes the other direction. WASI becomes an adapter into Wanix-owned semantics."

Concretely: when a guest opens a file, reads a directory, writes stdout, renames a path, creates a symlink, polls readiness, or reads fd flags, "that operation flows through Wanix namespace and fd policy" (`docs/rust-vs-go-wanix.md:235-237`). The guest still sees familiar APIs — `qjs:std`, `qjs:os`, `scriptArgs`, stdin/stdout/stderr, environment variables — "but the authority boundary is Wanix" (`docs/rust-vs-go-wanix.md:238-240`). The crate that owns this is `wanix-wasi`: it "owns the Preview 1 syscall semantics used by Rust Wanix tasks" and is "backed by Wanix namespaces, task fds, explicit preopens, and Wanix filesystem traits rather than by host WASI filesystem semantics" (`docs/adrs/0002-quickjs-wasi-task-runtime.md:47-52`). See [Wanix-backed WASI](/concepts/wanix-backed-wasi).

You see the boundary directly when you want a host directory inside a guest:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

# The host path is NOT ambient. It enters the guest only as an explicit mount.
wanix-rust qjs --mount /host/project=workspace main.js
```

The guest sees `workspace/...`, a Wanix namespace path. The host path `/host/project` is not authority the guest holds; it is authority you *granted* by binding it (`docs/rust-vs-go-wanix.md:242-255`). That distinction is minor for a local demo and load-bearing for cloud execution: you say *this* task gets *this* namespace, *these* mounts, *these* fds — and "the host filesystem" does not leak in because it happened to be convenient. See [host, not ambient, authority](/concepts/host-not-ambient-authority).

## Under the hood: the compiled-artifact cache as a trust boundary

Cranelift-compiling the ~1.7 MiB QuickJS fixture costs hundreds of milliseconds, which would dominate task cold-start. Wasmtime can serialize a compiled module to a host- and version-bound artifact and `deserialize` it in well under a millisecond, so `wanix-module-cache` caches the `.cwasm` keyed by the wasm SHA-256 (`crates/wanix-module-cache/src/lib.rs:1-16`). Both WASI runtimes share this single implementation.

The subtlety is that this cache is itself a trust boundary, not a convenience. "Deserializing a Wasmtime artifact is arbitrary-code-execution-equivalent: the SHA-256 key authenticates the *input wasm*, not the cached `.cwasm` bytes" (`crates/wanix-module-cache/src/lib.rs:22-26`). A local attacker who could pre-seed a `<sha256>.cwasm` would get code execution in the victim's process. So the cache refuses any directory that is not owner-private, and reads the artifact through file descriptors — opening the leaf dir `O_NOFOLLOW | O_DIRECTORY`, `fstat`ing it for owner and permission bits, then opening the artifact relative to that fd with `O_NOFOLLOW` and reading from the same fd — so no path is re-resolved between check and read (no symlink/TOCTOU window) (`crates/wanix-module-cache/src/lib.rs:28-44`). A directory or artifact that fails any check is treated as a cache miss and recompiled, never trusted (`crates/wanix-module-cache/src/lib.rs:45-46`). See [the compiled-artifact cache](/concepts/compiled-artifact-cache).

## Status / honest limits

- **Per-room memory isolation is real; hard CPU/memory limits are not.** Each Wasmtime instance is its own linear memory, so one guest cannot read another's heap — that isolation exists today. But the preemption and capping that would make a room safe for *arbitrary untrusted* code (Wasmtime epoch/fuel interruption, linear-memory caps) are not wired up yet (`docs/scaling-eli5.md:89-95`). The honest public claim is "cheap, scalable isolation," not "safe for arbitrary untrusted code." See [rooms, not houses](/concepts/rooms-not-houses) and [safe-for-untrusted is not yet claimable](/concepts/safe-for-untrusted-not-claimable).
- **The exec tiers on this substrate are local-trust only.** Devices that *run* code — `#task`, `#cpu`, `#agent` — are not exposed to untrusted or public peers. The substrate gives cheap isolation per instance; it does not yet give the resource bounds you would need before letting a hostile peer start tasks on your node.
- **WASI here is a command-style subset.** The compiled-wasm linker is a Preview 1 command subset; `poll_oneoff` is `NOSYS`. The substrate runs `_start` and records an exit; it is not a full async readiness runtime.

## See also

- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — the adapter that turns Preview 1 syscalls into namespace and fd operations.
- [Host, not ambient, authority](/concepts/host-not-ambient-authority) — why host paths enter only through explicit mounts.
- [Two tiers, one substrate](/concepts/two-tiers-one-substrate) — interpreted qjs and compiled wasm on the same engine.
- [The compiled-artifact cache](/concepts/compiled-artifact-cache) — the owner-private, fd-checked module cache.
- [Rooms, not houses](/concepts/rooms-not-houses) — the isolation model and where its limits sit.
- [Safe-for-untrusted is not yet claimable](/concepts/safe-for-untrusted-not-claimable) — the honest boundary on running hostile code.
- Prerequisite: [Everything is a file](/concepts/everything-is-a-file).
