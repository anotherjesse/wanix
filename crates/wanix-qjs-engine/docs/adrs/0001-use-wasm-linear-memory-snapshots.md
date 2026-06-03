# ADR 0001: Use WebAssembly Linear Memory Snapshots

## Status

Accepted

## Context

The project goal is a snapshot-able and resume-able JavaScript environment for
Rust. Native QuickJS embeds store engine pointers as process addresses, which
are not meaningful after process restart or in a fresh runtime instance. The
Vercel Labs reference demonstrates a different path: compile QuickJS to a
wasm32 WASI reactor so QuickJS pointers become offsets into WebAssembly linear
memory.

## Decision

Run QuickJS inside a WASI Preview 1 WebAssembly reactor and snapshot the VM as a
memory image: the full linear memory plus `__stack_pointer`, `JSRuntime*`, and
`JSContext*`.

## Consequences

- Snapshot/restore can preserve object graphs, closures, atoms, promise jobs,
  and pending promises without semantic JavaScript serialization.
- Snapshots are VM images, not portable JavaScript values.
- A snapshot is only valid for a compatible wasm module build and host import
  contract.
- Host resources such as Rust callbacks, files, sockets, timers, futures, and
  external state are not captured automatically and must be modeled or
  reattached by the host.
