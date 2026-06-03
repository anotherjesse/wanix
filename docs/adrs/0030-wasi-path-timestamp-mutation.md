# ADR 0030: WASI Path Timestamp Mutation

## Status

Accepted

## Context

QuickJS exposes `os.utimes(path, atime, mtime)`, which reaches WASI Preview 1
`path_filestat_set_times`. Wanix already owns QuickJS path open, stat, create,
remove, rename, and fdflag semantics through live `QuickJsWasiHost` providers.
Timestamp mutation should follow that same boundary: the engine copies guest
memory and raw Preview 1 values, while Wanix decides namespace resolution,
rights, metadata, and host clock policy.

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
unknown or contradictory flags as `INVAL`, and rejects `*_NOW` flags for now
because Wanix has not yet accepted a clock policy for this syscall.

Extend `QuickJsWasiHost` with `path_filestat_set_times`. The engine crate reads
the guest path from Wasmtime memory, forwards raw Preview 1-shaped arguments to
the live provider, and continues to return `NOSYS` for the read-only virtual
filesystem fallback.

## Consequences

JavaScript running as a Wanix `qjs` task can call `qjs:os.utimes(...)` and see
updated `atime`/`mtime` values through `qjs:os.stat(...)`.

Open-fd timestamp mutation (`fd_filestat_set_times`), `*_NOW` clock semantics,
symlink-specific timestamp behavior, and richer ctime policy remain follow-up
work. Snapshot bytes still contain only QuickJS/Wasm memory; Wanix reattaches
live timestamp-capable host resources through restore options.
