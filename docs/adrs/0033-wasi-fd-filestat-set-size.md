# ADR 0033: WASI FD Size Mutation

## Status

Accepted

## Context

WASI Preview 1 exposes file-size mutation through `fd_filestat_set_size`.
Wanix already supports create/truncate-at-open behavior, but there was no
post-open file length operation in the filesystem contract or live WASI
provider path.

This is different from a QuickJS-visible `truncate` demo. The bundled QuickJS
fixture does not currently expose `truncate` or `ftruncate`, so this cycle
implements the Wanix-owned syscall boundary and keeps JavaScript demo claims
limited to existing `qjs:std`/`qjs:os` capabilities.

## Decision

Add `File::set_len` to `wanix-fs` with an unsupported default, and implement it
for `MemFs` and explicit host-directory `LocalFs` file handles. The operation
resizes regular files while preserving the open fd offset.

Add Preview 1 `FD_FILESTAT_SET_SIZE` rights to `wanix-wasi`, include it on
write-capable regular-file fds, and implement `WasiCtx::fd_filestat_set_size`.
The operation requires a write-capable regular-file handle with the explicit
fd-size right. Stdio, preopen, and directory fds remain not-capable.

Add `QuickJsWasiHost::fd_filestat_set_size` and wire the engine import so live
providers receive the raw fd and size. The read-only virtual engine filesystem
continues to return `NOSYS`.

## Consequences

Wasmtime-hosted WASI guests can now resize open Wanix-backed regular files
through live providers. The `wanix-qjs` adapter forwards the syscall to
`WasiCtx`, keeping Wanix task/fd/namespace semantics outside the engine crate.

Guest JavaScript truncate/ftruncate demos remain future work until the bundled
QuickJS fixture exposes an API that reaches this Preview 1 import.
