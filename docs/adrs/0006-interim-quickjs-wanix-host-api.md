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
Wanix.open(path, mode) -> number
Wanix.readFd(fd, len) -> string
Wanix.writeFd(fd, text) -> number
Wanix.closeFd(fd) -> undefined
```

Path functions resolve ordinary paths relative to the task-start working
directory and keep `#task` service paths rooted in the task namespace. The task
context functions are sourced from a task-start snapshot of the Wanix `cmd`,
`env`, and `dir` task fields. Fd helpers allocate, read, write, and close
entries in the Wanix task fd table; they do not create a separate QuickJS fd
namespace. Because the current QuickJS callback surface copies scalar values,
`readFd` and `writeFd` are text/UTF-8 helpers rather than byte-accurate WASI
replacements. After an exit request, later host-visible output, namespace
writes, fd side effects, and pending jobs are suppressed. Console output
continues to flow through task stdout and stderr fds. This API is an
adapter-level bridge, not the final system call surface.

## Consequences

- The first QuickJS task driver can prove JavaScript has live access to Wanix
  namespace semantics before the full WASI import replacement is complete.
- `wanix-wasi` remains the target for durable Preview 1 filesystem and fd
  semantics.
- QuickJS can observe Wanix-owned task context without owning process identity
  or global fd semantics.
- QuickJS tasks can demonstrate Wanix-owned fd allocation before full
  Wanix-backed WASI Preview 1 imports are connected to the QuickJS guest.
- `Wanix.exit` was removed after `qjs:std.exit(...)` reached Wanix-owned WASI
  `proc_exit` through the live provider.
- The `Wanix` JavaScript object may be removed, renamed, or moved behind a
  compatibility flag once QuickJS receives Wanix-backed WASI imports directly.
