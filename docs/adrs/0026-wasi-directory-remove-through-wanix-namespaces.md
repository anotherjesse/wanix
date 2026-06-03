# ADR 0026: WASI Directory Remove Through Wanix Namespaces

## Status

Accepted.

## Context

Wanix-backed QuickJS/WASI tasks can now create directories through
`PATH_CREATE_DIRECTORY`, but cleanup still depended on the engine's unsupported
`path_remove_directory` fallback. QuickJS exposes directory removal through
`qjs:os.remove(...)`, which uses the WASI directory-removal import for directory
targets.

Directory removal crosses the same public trust boundary as directory creation:
the backing filesystem decides whether a path is an empty directory, the
namespace decides which bind target is visible, `wanix-wasi` owns rights and
errno projection, and the engine must only forward guest memory to a live
provider.

## Decision

Add exact empty-directory removal to the shared filesystem contract with
`FileSystem::remove_dir`. `MemFs` and `LocalFs` remove one empty non-root
directory and reject non-empty directories, non-directory targets, missing
targets, and attempts to remove the filesystem root. `LocalFs` validates the
resolved directory stays inside its configured host root before removal.

`Namespace` routes `remove_dir` through resolved bind targets. Synthetic
namespace-only directories remain namespace structure rather than mutable
backing directories, so removal reports them as non-empty.

Add `WasiRights::PATH_REMOVE_DIRECTORY` and map `FsError::NotEmpty` to the
Preview 1 `NOTEMPTY` errno. Expose
`WasiCtx::path_remove_directory(dirfd, path)`, requiring the directory fd to
carry `PATH_REMOVE_DIRECTORY`.

Extend the live QuickJS WASI host boundary with `path_remove_directory`. The
engine forwards the Preview 1 import to the live provider when present, while
the read-only virtual-WASI fallback remains unsupported. `wanix-qjs` delegates
the provider method to `wanix_wasi::WasiCtx`.

## Consequences

Guest JavaScript can use `qjs:os.remove(...)` to remove empty directories in
native `wanix-rust qjs` demos, while non-empty directories surface a real WASI
`NOTEMPTY` error instead of generic I/O.

This does not add recursive removal, rename, timestamp mutation, or fd flag
mutation. Those remain separate decisions because each has different namespace
and host-resource semantics.
