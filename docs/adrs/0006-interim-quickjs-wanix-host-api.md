# ADR 0006: Interim QuickJS Wanix Host API

## Status

Superseded. The `globalThis.Wanix` bridge has been removed.

## Context

The `rust-wasi-quickjs` prototype provides a proven Wasmtime-hosted QuickJS
module and host callback surface, but it does not yet delegate WASI filesystem
syscalls to `wanix-wasi`. The first Wanix Rust demo still needs JavaScript to
observe and mutate a Wanix task namespace outside Chrome.

## Decision

`wanix-qjs` exposed a narrow temporary JavaScript object,
`globalThis.Wanix`, for the first vertical slice:

```text
Wanix.readText(path)
Wanix.writeText(path, text)
Wanix.args()
Wanix.env(...)
Wanix.cwd()
Wanix.cmd()
Wanix.exit(code)
Wanix.open(path, mode)
Wanix.readFd(fd, len)
Wanix.writeFd(fd, text)
Wanix.closeFd(fd)
```

Guest code should use `qjs:std`, `qjs:os`, `scriptArgs`, and `#task` service
files instead. Dynamic file descriptors opened through `qjs:os.open(...)` are
mirrored into the Wanix task fd table by the live `wanix-wasi` provider, so
QuickJS still observes Wanix-owned fd semantics without a separate JavaScript
bridge object. Console output continues to flow through task stdout and stderr
fds. The removed API was an adapter-level bridge, not the final system call
surface.

## Consequences

- The first QuickJS task driver can prove JavaScript has live access to Wanix
  namespace semantics before the full WASI import replacement is complete.
- `wanix-wasi` remains the target for durable Preview 1 filesystem and fd
  semantics.
- QuickJS can observe Wanix-owned task context without owning process identity
  or global fd semantics.
- `Wanix.exit` was removed after `qjs:std.exit(...)` reached Wanix-owned WASI
  `proc_exit` through the live provider.
- `Wanix.readText`, `Wanix.writeText`, `Wanix.args`, `Wanix.env`, `Wanix.cwd`,
  and `Wanix.cmd` were removed after guest code could use live WASI path/env/arg
  calls and `#task` service files directly.
- `Wanix.open`, `Wanix.readFd`, `Wanix.writeFd`, and `Wanix.closeFd` were
  removed after `qjs:os` fd calls could mirror dynamic fds into the Wanix task
  fd table and snapshot blockers could report open dynamic WASI fds.
