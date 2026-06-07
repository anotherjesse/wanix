---
title: Two Tiers, One Substrate
slug: concepts/two-tiers-one-substrate
pageType: concept
oneLiner: Interpreted (qjs) and compiled (wasm) tasks built from one Namespace share a filesystem — a write by one is visible to the other.
audience: [visionary, developer]
tags: [tasks, runtime, wasmtime, shipped]
sourceRefs:
  - crates/wanix-wasm/src/lib.rs:1-31
  - crates/wanix-cli/tests/shared_vfs_differential.rs:1-126
  - crates/wanix-cli/tests/shared_vfs_differential.rs:520-566
  - docs/scaling-eli5.md:40-66
seeAlso:
  - concepts/qjs-task
  - concepts/compiled-wasm-task-driver
  - concepts/rooms-not-houses
  - concepts/shared-wasi-fd-contract
prerequisites:
  - concepts/qjs-task
  - concepts/compiled-wasm-task-driver
usedInFlows: []
honestLimits:
  - "The differential proof is over fixtures (MemFs, a tiny rust-guest.wasm); real-world workloads beyond demos are not demonstrated in-tree."
  - "qjs is an interpreter — heavy number-crunching is slower than compiled wasm; the speedup claim is the project's own framing, not a benchmarked figure here."
  - "There are no hard CPU/memory limits on tasks yet; exec devices are local-trust only."
---

# Two Tiers, One Substrate

Interpreted (qjs) and compiled (wasm) tasks built from one Namespace share a filesystem — a write by one is visible to the other.

You should not have to pick your isolation story when you pick your language. In Wanix you pick the *engine* per task — easy-and-interpreted JavaScript or fast-and-compiled WebAssembly — and the security story stays identical, because both run on the same Wasmtime substrate over the same Wanix `Namespace`. The proof is concrete and in-tree: a qjs task and a Rust `wasm32-wasi` task, built from one `MemFs`, observe byte-for-byte identical filesystem state. The tier is a knob, not a fork in the architecture.

## Easy or fast, same room

Two task drivers sit on the same substrate. The `qjs` task is a QuickJS interpreter: write JavaScript, no build step, perfect for glue code and small or AI-generated snippets (`docs/scaling-eli5.md:59-60`). The compiled-wasm task is a `wasm32-wasi` command module — Rust, Go, C, or Zig compiled ahead of time and run near-native (`crates/wanix-wasm/src/lib.rs:1-7`). The crate's own header names them siblings: the wasm runner is "the 'compiled-to-wasm task' sibling of the QuickJS `qjs` task: same sandbox, same namespace/VFS, near-native speed."

The choice is per task, and nothing about the room changes when you make it. Both engines are guests of Wasmtime. Both reach the outside world only through WASI syscalls. Both have those syscalls backed by a Wanix `Namespace` through one engine-agnostic `WasiCtx` (`crates/wanix-wasm/src/lib.rs:4-7`). So a QuickJS snippet and a compiled hot-path binary live in the same kind of locked room with the same doors; the only difference between them is how fast the code inside runs.

## One Namespace, one filesystem

The load-bearing claim is in the wasm crate's doc comment, stated flatly: "Two tasks (a `qjs` task and a `wanix-wasm` task) that are built from the same `Namespace` share one filesystem — writes by one are visible to the other" (`crates/wanix-wasm/src/lib.rs:9-10`).

This is not coincidence; it is mechanism. A task's view of the world *is* its `Namespace`. If you bind the same backing `FileSystem` into two namespaces — or hand both tasks the same namespace — then both engines resolve, read, and write through one VFS. There is no per-engine filesystem shim, no "JavaScript files" versus "wasm files." A path is a path, and both tiers route the same operation through the same backend.

## The differential proof

The `shared_vfs_differential` test suite makes the claim falsifiable (`crates/wanix-cli/tests/shared_vfs_differential.rs`). Each test binds one `MemFs` at the namespace root, then runs a real QuickJS task and a real compiled `rust-guest.wasm` task against it. The header states the contract: both engines "must observe identical filesystem state."

The two-way write test is the cleanest demonstration (`crates/wanix-cli/tests/shared_vfs_differential.rs:520-566`). The qjs task writes `/shared/from_qjs.txt`. The rust-wasm task reads that file, then writes `/shared/from_rust.txt`. The qjs task reads *that* back and gets `rust-wasm saw: hello from qjs`. Then the test reaches straight into the backing `MemFs` and confirms both files are present with exactly those bytes. JavaScript wrote, compiled code read and answered, JavaScript read the answer — one filesystem, two engines, no bridge.

The directory-listing test is a true differential (`crates/wanix-cli/tests/shared_vfs_differential.rs:86-125`): qjs (`os.readdir` + `os.lstat`) and rust-wasm (`std::fs::read_dir`) each emit a sorted `name type` listing of the same directory, and the two must match byte-for-byte — proving both route through the same `WasiCtx::fd_read_dir` backend. Companion tests carry rename, rmdir (including an `ENOTEMPTY` rejection), symlink and readlink in both directions, `fd_tell` after a seek, and truncate — each one a qjs/wasm pair agreeing on the same shared VFS. The cockpit ships a live version of this as the qjs to wasm to qjs duet on one shared FS.

## Pick the engine in the same secure room

Because the room is the same, the tier becomes a pure cost decision. The project frames it as three rungs (`docs/scaling-eli5.md:57-66`): quick-and-easy JavaScript with no build step; fast compiled Rust/Go/etc. for hot paths, near-native speed in the same secure room; and, when you need a whole OS, a full VM. "Same security story across all three — you just dial in how much speed you need." You can start a feature in qjs for the iteration speed and drop the inner loop into a `.wasm` task later without re-reasoning about the sandbox, the namespace, or who can read your files.

## See also

- [The qjs task](/concepts/qjs-task) — the interpreted tier.
- [The compiled wasm task driver](/concepts/compiled-wasm-task-driver) — the compiled tier.
- [Rooms, not houses](/concepts/rooms-not-houses) — why cheap, identical isolation is the point.
- [The shared WASI fd contract](/concepts/shared-wasi-fd-contract) — the fd/stdio path both tiers follow.

## Status / honest limits

- The differential proof runs over fixtures: an in-memory `MemFs` and a small `rust-guest.wasm`. It proves the two tiers share one VFS and agree on filesystem semantics; it does not demonstrate real-world workloads beyond demos and tests.
- qjs is an interpreter, so heavy number-crunching is slower than compiled wasm. The "near-native / faster engine" framing is the project's own (`docs/scaling-eli5.md:42-55`), not a benchmarked figure on this page.
- The wasm tier is command-style WASI: the shared linker registers `poll_oneoff` as `ERRNO_NOSYS`, so guests get `_start`, fd/path I/O, args/env, clock, and exit, but no poll readiness — it is not a general-purpose WASI host (`crates/wanix-wasm/src/lib.rs:12-17`).
- "Same security story" means the same kind of sandbox, not arbitrary-untrusted-code safety. There are no hard CPU/memory limits on tasks yet, and the exec devices that start tasks are local-trust only.
