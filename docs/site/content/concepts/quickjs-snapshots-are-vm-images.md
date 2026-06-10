---
title: QuickJS Snapshots Are VM Images
slug: concepts/quickjs-snapshots-are-vm-images
pageType: concept
oneLiner: A snapshot captures WebAssembly linear memory plus the QuickJS pointers into it, not serialized task state, so live Wanix host state (namespace, fds, cwd, env, providers) must be reattached on restore.
audience: [developer]
tags: [qjs, wasi, snapshot, caveat, cli, shipped]
sourceRefs:
  - crates/wanix-qjs-engine/docs/architecture.md:19-62
  - docs/adrs/0002-quickjs-wasi-task-runtime.md:71-81
  - rust-walkthrough.md:481-575
  - crates/wanix-cli/src/qjs.rs:56-83
  - crates/wanix-cli/src/lib.rs:3135-3161
seeAlso: [concepts/qjs-task, concepts/wanix-capsule, concepts/tasks-own-process-identity]
prerequisites: [concepts/qjs-task]
usedInFlows: []
honestLimits:
  - A snapshot is bound to the exact QuickJS WASM module SHA-256; restore against a different fixture is rejected, not migrated.
  - Snapshots store no host state — namespace, mounts, cwd, argv/env, stdio, task fds, and live WASI providers must be supplied again at restore.
  - Open dynamic descriptors block a snapshot; there is no serializable virtual-fd format yet.
canonicalCaveatFor: []
---

# QuickJS Snapshots Are VM Images

A snapshot captures WebAssembly linear memory plus the QuickJS pointers into it, not serialized task state, so live Wanix host state (namespace, fds, cwd, env, providers) must be reattached on restore.

When you "snapshot" a running QuickJS task, the natural mental model is wrong in a useful way. You are not serializing a JavaScript heap into JSON, and you are not checkpointing a Wanix task. You are freezing a *virtual machine* — the WebAssembly linear memory of the QuickJS reactor and the saved pointers into it — and writing it to bytes. That distinction is the whole page: it explains what comes back (your JS world) and what does not (everything Wanix was holding on the outside).

## Show it: the VM survives, the task does not

Run the bundled restore demo. The `before` script writes a global, Wanix snapshots the VM image, and the `after` script runs in a *new* task with host resources reattached around the same memory (`crates/wanix-cli/src/lib.rs:3135-3161`):

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

wanix qjs-restore examples/qjs-snapshot-before.js examples/qjs-snapshot-after.js
echo "exit=$?"
```

```text
before task: 1
after task: 2
vm state: preserved from task 1
namespace: namespace from task 1
exit=7
```

Read that output carefully. `vm state: preserved from task 1` — the JavaScript global the `before` script set is still there. But `after task: 2` — the task id changed from `1` to `2`. The VM came back; the task did not. The namespace line printed "from task 1" only because the demo *reattached* a namespace that says so, not because the snapshot carried one. This is the Plan 9-adjacent term Wanix uses for the artifact: a **VM image**, not a task checkpoint (`docs/adrs/0002-quickjs-wasi-task-runtime.md:71`).

## What is captured, and what is not

QuickJS runs as a WASI Preview 1 WebAssembly reactor. Native QuickJS pointers are not portable across a process boundary — but *inside* wasm32 they are just offsets into linear memory, and linear memory is a flat byte array you can copy. That is precisely why a VM image works where JS serialization would be a nightmare (`crates/wanix-qjs-engine/docs/architecture.md:19-23`).

A snapshot's v1 envelope therefore contains exactly the VM, and nothing of the host (`crates/wanix-qjs-engine/docs/architecture.md:24-49`):

- a versioned little-endian header;
- the QuickJS WASM ABI version and the exact module SHA-256;
- the WebAssembly linear-memory image;
- the saved `__stack_pointer`;
- the saved QuickJS runtime and context pointers.

What is deliberately *absent* is everything Wanix owns on the Rust side. Snapshots do not serialize the task id, the namespace, mounts, cwd, argv/env, stdio capture, task fds, clocks or random policy, terminal attachments, execution limits, exit observation, module loaders, promise-rejection handlers, interrupt closures, or live WASI providers (`crates/wanix-qjs-engine/docs/architecture.md:59-62`, `docs/adrs/0002-quickjs-wasi-task-runtime.md:71-74`). The clean line: if it lives in QuickJS's linear memory, it is in the image; if Wanix is holding it, it is not.

## Restore reattaches host state explicitly

Because the image is host-free, restore is not "resume the task" — it is "build a fresh instance, copy the validated memory in, then hand it a *new* set of host resources." Live WASI providers are attached through `QuickJsRestoreOptions::with_wasi_host`, deliberately kept out of the cloneable deterministic config so the live, mutable handles are reattached every time rather than baked into a fixture (`crates/wanix-qjs-engine/docs/architecture.md:64-74`).

This is why the task id ticked from `1` to `2`: Wanix minted a new task and wrapped the restored memory in it. The persisted form makes the same point across CLI invocations. Snapshot to a file, then resume into a *different* host environment (`rust-walkthrough.md:512-575`):

```sh
mkdir -p /tmp/wanix-persist
SNAP=/tmp/wanix-persist/quickjs.snapshot

wanix qjs-snapshot --env MODE=before --mount /tmp/wanix-persist=host \
  --snapshot "$SNAP" examples/qjs-persist-before.js
# snapshot task: 1

wanix qjs-resume --env MODE=after --mount /tmp/wanix-persist=host \
  --snapshot "$SNAP" examples/qjs-persist-after.js
echo "exit=$?"
```

```text
resume task: 1
vm: vm from task 1 mode before
reattached mode: after
host: host before task 1
exit=6
```

The VM remembers `mode before` from when it was frozen; the *host* sees `mode after` because `--env` and `--mount` were supplied again on resume. The snapshot did not store the env var or the mount as an ambient capability — they are host policy, reattached, not baked in (`rust-walkthrough.md:572-575`). The subcommands themselves are `qjs-snapshot`, `qjs-resume`, and the in-process `qjs-restore` (`crates/wanix-cli/src/qjs.rs:56-83`).

## Open dynamic descriptors block a snapshot

There is one more rule that keeps the boundary honest: a task with open *dynamic* descriptors cannot be snapshotted, unless and until a future decision defines a serializable virtual-fd state (`docs/adrs/0002-quickjs-wasi-task-runtime.md:74-76`). This is not a missing feature so much as a refusal to lie. An fd into a live pipe, a remote 9P file, or a terminal is a handle to host state that the image cannot carry; rather than silently dropping it and restoring a VM that thinks it holds a connection it does not, Wanix declines the snapshot. The boundary stays visible.

## Under the hood

Restore validates the envelope and the module identity *before* it copies memory into a fresh instance (`crates/wanix-qjs-engine/docs/architecture.md:54-57`). Raw guest pointers stay private — public metadata exposes only route-friendly compatibility fields (format version, ABI version, module hash, memory length), never borrowed guest views or raw `JSValue*` handles (`crates/wanix-qjs-engine/docs/architecture.md:80-96`). Numeric QuickJS limits may live inside VM memory and thus ride along in the image, but required host policy (clocks, random, interrupts) is reapplied after every create and restore, because Rust closures are host state and must be reattached (`crates/wanix-qjs-engine/docs/architecture.md:98-111`).

This is the same VM-image discipline that a [Wanix capsule](/concepts/wanix-capsule) extends from one task to a whole world: freeze the portable bytes, leave the live, ephemeral handles behind.

## See also

- [The qjs task](/concepts/qjs-task) — how a QuickJS script becomes a Wanix task in the first place.
- [Wanix capsule](/concepts/wanix-capsule) — freezing an entire world to portable, CAS-backed bytes (live peers and ephemeral handles do not travel).
- [Tasks own process identity](/concepts/tasks-own-process-identity) — why restore mints a new task id rather than reviving the old one.

## Status / honest limits

- **Bound to one fixture.** A snapshot is keyed to the exact QuickJS WASM SHA-256. Restore against a different fixture (a rebuilt QuickJS C adapter, changed compiler flags, a new helper set) is *rejected*, not migrated (`crates/wanix-qjs-engine/docs/architecture.md:54-57`, `:113-119`).
- **No host state inside.** Namespace, mounts, cwd, argv/env, stdio, task fds, and live WASI providers are never in the image; you must supply them again on restore (`docs/adrs/0002-quickjs-wasi-task-runtime.md:71-74`).
- **Open dynamic fds block snapshot.** There is no serializable virtual-fd format yet, so a task holding dynamic descriptors cannot be frozen (`docs/adrs/0002-quickjs-wasi-task-runtime.md:74-76`).
