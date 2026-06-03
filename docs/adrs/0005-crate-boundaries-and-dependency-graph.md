# ADR 0005: Crate Boundaries and Dependency Graph

## Status

Accepted

## Context

The Rust port should recreate Wanix contracts without copying the Go package
shape blindly. The crate graph needs to keep low-level filesystem and namespace
contracts independent from Wasmtime and QuickJS so tests and future runtimes can
use them directly.

## Decision

Use this initial dependency direction:

```text
wanix-fs
  -> wanix-vfs
  -> wanix-task

wanix-wasi -> wanix-fs + wanix-vfs
wanix-qjs  -> wanix-task + wanix-wasi + rust-wasi-quickjs
wanix-cli  -> runtime crates for orchestration
```

`wanix-task` must not depend on `wanix-wasi` or `wanix-qjs`. QuickJS task
registration lives in `wanix-qjs` and is wired by the CLI or another
composition layer.

## Consequences

- `wanix-fs` and `wanix-vfs` remain testable without Wasmtime.
- `wanix-wasi` can evolve as a generic Wanix-backed WASI adapter, not a
  QuickJS-specific layer.
- `wanix-qjs` can preserve the `rust-wasi-quickjs` lifecycle ergonomics,
  implement the task-driver adapter, and keep raw guest memory and QuickJS
  handles private.
- Future `wanix-protocol` should hold wire DTOs and protocol codecs, with
  adapters at crate edges instead of protocol types leaking into core state.
