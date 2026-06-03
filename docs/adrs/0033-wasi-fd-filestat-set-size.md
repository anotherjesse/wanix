# ADR 0033: WASI FD Size Mutation

## Status

Accepted

## Context

WASI Preview 1 exposes file-size mutation through `fd_filestat_set_size`.
Wanix already supports create/truncate-at-open behavior, but there was no
post-open file length operation in the filesystem contract or live WASI
provider path.

This was initially different from a QuickJS-visible `truncate` demo. At the
time of this ADR, the bundled QuickJS fixture did not expose `truncate` or
`ftruncate`, so this cycle implemented the Wanix-owned syscall boundary and kept
JavaScript demo claims limited to existing `qjs:std`/`qjs:os` capabilities.

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

ADR 0039 later exposes `qjs:os.truncate` and `qjs:os.ftruncate`, making this
Preview 1 import directly reachable from guest JavaScript.
