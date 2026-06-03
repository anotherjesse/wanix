# ADR 0002: Wanix-Owned WASI Semantics

## Status

Accepted

## Context

Wanix filesystem behavior includes virtual resources, per-task namespaces,
Plan 9-style bind and union resolution, `#task`, pipes, signals, terminals,
and other capability-oriented services. Generic host WASI filesystem adapters
usually model host paths and preopens, which would hide or flatten Wanix
semantics at the trust boundary.

## Decision

The Rust port will provide custom WASI Preview 1 imports backed by Wanix
namespaces and task file descriptors. Wasmtime remains the engine, but Wanix
owns syscall semantics for filesystem and fd behavior.

## Consequences

- WASI `path_*` and `fd_*` calls resolve through Wanix task state.
- Host paths are exposed only through explicit Wanix resources such as a future
  local filesystem adapter.
- Early WASI support can start narrow and read-only, but the API boundary should
  be designed for full Wanix filesystem behavior.
- `wanix-wasi` depends on `wanix-fs` and `wanix-vfs`; task-fd attachment is
  wired by `wanix-cli`, `wanix-qjs`, or another composition layer so the core
  crate graph stays acyclic.
