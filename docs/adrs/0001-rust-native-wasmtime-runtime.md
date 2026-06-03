# ADR 0001: Rust-Native Wasmtime Runtime

## Status

Accepted

## Context

Wanix currently runs its primary runtime through Go compiled to browser wasm,
with browser and `syscall/js` assumptions close to the core execution model.
The Rust port should run outside Chrome and make browser support a frontend or
deployment option rather than the foundation.

The `rust-wasi-quickjs` prototype proves that a QuickJS runtime can run as a
WASI Preview 1 WebAssembly reactor under Wasmtime, with host policy supplied by
Rust and VM state captured through linear-memory snapshots.

## Decision

Rust Wanix will act as the host/microkernel. Wasmtime will be the execution
substrate for guest programs. QuickJS/WASI will be the first serious task
runtime for the port, and the first externally visible demo target is:

> Run JavaScript outside Chrome with access to a Wanix namespace.

## Consequences

- Browser APIs are not part of the core runtime contract.
- Wasmtime integration is core infrastructure, not an optional adapter.
- The Go implementation remains a behavior oracle during migration.
- The Rust port should recreate Wanix contracts instead of translating Go files
  package by package.

## First Slice Acceptance Criteria

The first vertical slice should run JavaScript outside Chrome, load source from
a Wanix namespace, expose a small JS-visible Wanix filesystem API or module,
write output through task stdio, and return an observable exit status.
