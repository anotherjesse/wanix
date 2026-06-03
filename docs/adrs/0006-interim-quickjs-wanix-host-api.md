# ADR 0006: Interim QuickJS Wanix Host API

## Status

Partially superseded. The namespace, task context, and exit helpers have been
removed. Only the fd helpers remain as a temporary legacy bridge.

## Context

The `rust-wasi-quickjs` prototype provides a proven Wasmtime-hosted QuickJS
module and host callback surface, but it does not yet delegate WASI filesystem
syscalls to `wanix-wasi`. The first Wanix Rust demo still needs JavaScript to
observe and mutate a Wanix task namespace outside Chrome.

## Decision

`wanix-qjs` exposed a narrow temporary JavaScript object,
`globalThis.Wanix`, for the first vertical slice. The remaining live surface is:

```text
Wanix.open(path, mode) -> number
Wanix.readFd(fd, len) -> string
Wanix.writeFd(fd, text) -> number
Wanix.closeFd(fd) -> undefined
```

The removed helpers were:

```text
Wanix.readText(path)
Wanix.writeText(path, text)
Wanix.args()
Wanix.env(...)
Wanix.cwd()
Wanix.cmd()
Wanix.exit(code)
```

Guest code should use `qjs:std`, `qjs:os`, `scriptArgs`, and `#task` service
files instead. Fd helpers allocate, read, write, and close entries in the Wanix
task fd table; they do not create a separate QuickJS fd namespace. Because the
current QuickJS callback surface copies scalar values, `readFd` and `writeFd`
are text/UTF-8 helpers rather than byte-accurate WASI replacements. After an
exit request, later host-visible output, fd side effects, and pending jobs are
suppressed. Console output continues to flow through task stdout and stderr fds.
This API is an adapter-level bridge, not the final system call surface.

## Consequences

- The first QuickJS task driver can prove JavaScript has live access to Wanix
  namespace semantics before the full WASI import replacement is complete.
- `wanix-wasi` remains the target for durable Preview 1 filesystem and fd
  semantics.
- QuickJS can observe Wanix-owned task context without owning process identity
  or global fd semantics.
- The remaining fd helpers preserve the early Wanix task-fd proof while qjs
  tests/examples migrate to `qjs:os` fd calls.
- `Wanix.exit` was removed after `qjs:std.exit(...)` reached Wanix-owned WASI
  `proc_exit` through the live provider.
- `Wanix.readText`, `Wanix.writeText`, `Wanix.args`, `Wanix.env`, `Wanix.cwd`,
  and `Wanix.cmd` were removed after guest code could use live WASI path/env/arg
  calls and `#task` service files directly.
- The `Wanix` JavaScript object may be removed, renamed, or moved behind a
  compatibility flag once QuickJS receives Wanix-backed WASI imports directly.
