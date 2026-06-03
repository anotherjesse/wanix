# ADR 0006: Interim QuickJS Wanix Host API

## Status

Accepted

## Context

The `rust-wasi-quickjs` prototype provides a proven Wasmtime-hosted QuickJS
module and host callback surface, but it does not yet delegate WASI filesystem
syscalls to `wanix-wasi`. The first Wanix Rust demo still needs JavaScript to
observe and mutate a Wanix task namespace outside Chrome.

## Decision

`wanix-qjs` will expose a narrow temporary JavaScript object,
`globalThis.Wanix`, for the first vertical slice:

```text
Wanix.readText(path) -> string
Wanix.writeText(path, text) -> undefined
Wanix.args() -> string[]
Wanix.env() -> object
Wanix.env(name) -> string | undefined
Wanix.cwd() -> string
Wanix.cmd() -> string
```

Path functions resolve ordinary paths relative to the task-start working
directory and keep `#task` service paths rooted in the task namespace. The task
context functions are sourced from a task-start snapshot of the Wanix `cmd`,
`env`, and `dir` task fields. Console output continues to flow through task
stdout and stderr fds. This API is an adapter-level bridge, not the final system
call surface.

## Consequences

- The first QuickJS task driver can prove JavaScript has live access to Wanix
  namespace semantics before the full WASI import replacement is complete.
- `wanix-wasi` remains the target for durable Preview 1 filesystem and fd
  semantics.
- QuickJS can observe Wanix-owned task context without owning process identity
  or global fd semantics.
- The `Wanix` JavaScript object may be removed, renamed, or moved behind a
  compatibility flag once QuickJS receives Wanix-backed WASI imports directly.
