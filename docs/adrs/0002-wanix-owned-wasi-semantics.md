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

`wanix-wasi` accepts standard fd attachments as generic `wanix_fs::File`
handles with explicit read/write access. Task-specific adapters, such as
`wanix-qjs`, are responsible for supplying task fd proxy files so the WASI crate
does not depend upward on `wanix-task`.

## Consequences

- WASI `path_*` and `fd_*` calls resolve through Wanix task state.
- Host paths are exposed only through explicit Wanix resources such as a future
  local filesystem adapter.
- Stdio defaults to closed; fd 0/1/2 become available only when the composition
  layer attaches explicit Wanix file handles.
- `wanix-wasi` exposes typed Preview 1 import metadata such as preopen names,
  fdstat file types/rights, and numeric errno codes without depending on
  Wasmtime guest memory. Engine-specific import providers translate those typed
  results and byte-layout encoders into guest ABI structs.
- Engine-specific providers translate guest-memory ABI records into typed host
  calls. Runtime adapters such as `wanix-qjs` convert those calls into
  `wanix-wasi` path/fd operations. Unsupported Preview 1 modes remain explicit
  errors until Wanix owns their semantics.
- Preview 1 `path_open` requests preserve reduced base and inheriting rights on
  opened file and directory fds. `fdstat` and later fd/path operations report
  and enforce those effective rights instead of re-advertising broader defaults.
- Directory fdstat rights should describe the operations Wanix already allows:
  directory reads, recursive path opens, file creation, and truncation during
  path open. Directory fds inherit both regular-file and child-directory rights.
- File handles advertise seek/tell rights only when the underlying Wanix file
  reports that capability. `fd_seek`/`fd_tell` are backed by the Wanix file
  contract for seekable fds, while non-seekable valid fds report insufficient
  capability and bad fds are still reported as `badf`.
- Early WASI support can start narrow and read-only, but the API boundary should
  be designed for full Wanix filesystem behavior.
- `wanix-wasi` depends on `wanix-fs` and `wanix-vfs`; task-fd attachment is
  wired by `wanix-cli`, `wanix-qjs`, or another composition layer so the core
  crate graph stays acyclic.
