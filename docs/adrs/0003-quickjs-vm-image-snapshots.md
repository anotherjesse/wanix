# ADR 0003: QuickJS VM Image Snapshots

## Status

Accepted

## Context

QuickJS pointers are process addresses in a native embed, but they are offsets
inside WebAssembly linear memory when QuickJS runs as a wasm32 WASI reactor.
The `rust-wasi-quickjs` prototype uses that property to snapshot and restore
the VM by copying memory plus the stack pointer and QuickJS runtime/context
pointers.

Wanix task state includes host-side resources that do not live in QuickJS
linear memory: namespaces, file descriptors, host callbacks, module loader
closures, clocks, random policy, sockets, and other resources.

## Decision

QuickJS snapshots in Rust Wanix are VM images. Host resources are reattached
explicitly when a task is restored. Initial Wanix snapshot support will reject
open file descriptors for QuickJS tasks; later versions may serialize selected
virtual fd state when the semantics are well-defined.

## Consequences

- Snapshot bytes are tied to a compatible QuickJS/WASI module build.
- Host policy is supplied on create and restore.
- Snapshotting must happen at well-defined yield points.
- Wanix task snapshots need separate metadata for task identity, namespace
  bindings, and restart policy instead of pretending those resources are inside
  QuickJS memory.
