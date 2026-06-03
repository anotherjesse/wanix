# ADR 0030: WASI Path and FD Timestamp Mutation

## Status

Accepted

## Context

QuickJS exposes `os.utimes(path, atime, mtime)`, which reaches WASI Preview 1
`path_filestat_set_times`. WASI guests may also mutate timestamps through an
already-open descriptor with `fd_filestat_set_times`. Wanix already owns QuickJS
path open, stat, create, remove, rename, and fdflag semantics through live
`QuickJsWasiHost` providers. Timestamp mutation should follow that same
boundary: the engine copies guest memory and raw Preview 1 values, while Wanix
decides namespace resolution, fd rights, metadata, and host clock policy.

`QuickJsHostConfig` remains deterministic runtime policy. Live filesystem
state, including timestamp mutation, stays attached through
`QuickJsCreateOptions::with_wasi_host(...)` and
`QuickJsRestoreOptions::with_wasi_host(...)`.

## Decision

Add access, modification, and metadata-change timestamps to
`wanix_fs::Metadata` as nanoseconds since the Unix epoch. Existing constructors
keep deterministic zero timestamps; filesystems can opt into explicit times
through `Metadata::new_with_times(...)`.

Add `FileSystem::set_times(path, atime_ns, mtime_ns)` as a path-level contract.
`MemFs` stores explicit atime/mtime on nodes and keeps ctime deterministic for
now. `LocalFs` uses `std::fs::FileTimes` and `File::set_times` after the same
root-preserving canonicalization used by other `LocalFs` operations.

Add `WasiRights::PATH_FILESTAT_SET_TIMES` and include it in directory base and
inheriting rights. `WasiCtx::path_filestat_set_times` resolves paths through the
task namespace, preserves timestamps whose Preview 1 flags are absent, rejects
unknown or contradictory flags as `INVAL`, and initially rejected `*_NOW` flags
until Wanix accepted the deterministic clock policy in ADR 0034.

Add `WasiRights::FD_FILESTAT_SET_TIMES` for open descriptors and include it in
regular file and directory fd rights. `WasiCtx::fd_filestat_set_times` mutates
the namespace path captured by the open fd, preserving the same explicit-only
timestamp policy as the path syscall. Attached stdio fds remain incapable of
timestamp mutation.

Extend `QuickJsWasiHost` with `path_filestat_set_times`. The engine crate reads
the guest path from Wasmtime memory, forwards raw Preview 1-shaped arguments to
the live provider, and continues to return `NOSYS` for the read-only virtual
filesystem fallback.

Extend `QuickJsWasiHost` with `fd_filestat_set_times`. The engine crate forwards
the raw fd/timestamp arguments to live providers and keeps the read-only virtual
filesystem fallback isolated by returning `NOSYS`.

## Consequences

JavaScript running as a Wanix `qjs` task can call `qjs:os.utimes(...)` and see
updated `atime`/`mtime` values through `qjs:os.stat(...)`. WASI guests with
`FD_FILESTAT_SET_TIMES` can also mutate timestamps through an open namespace fd
without delegating process or filesystem policy to QuickJS.

Symlink-specific timestamp behavior and richer ctime policy remain follow-up
work. Snapshot bytes still contain only QuickJS/Wasm memory; Wanix reattaches
live timestamp-capable host resources through restore options.
