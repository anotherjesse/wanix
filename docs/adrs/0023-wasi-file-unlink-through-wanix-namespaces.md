# ADR 0023: WASI File Unlink Through Wanix Namespaces

## Status

Accepted.

## Context

QuickJS/WASI tasks can create, write, read, seek, stat, and close regular files
through Wanix-backed WASI. Directory and file mutation beyond create/truncate
remained unsupported at the engine boundary, so ordinary JavaScript workflows
that remove generated files could not run through `qjs:os` outside Chrome.

File deletion crosses Wanix's public contracts: core filesystems need a
mutation operation, namespaces need to route that operation through bind
resolution, `wanix-wasi` needs to enforce Preview 1 rights, and the QuickJS
engine should only forward guest memory to a live provider rather than owning
Wanix policy.

## Decision

Add `FileSystem::remove_file` for non-directory file removal. `MemFs` and
`LocalFs` implement it directly, while `Namespace` routes it to the
highest-priority resolved binding and preserves existing directory/synthetic
directory errors.

Add `WasiRights::PATH_UNLINK_FILE` and expose
`WasiCtx::path_unlink_file(dirfd, path)`. The operation resolves paths through
the same Wanix namespace rules as other WASI path calls and requires the
directory fd to carry `PATH_UNLINK_FILE`.

Extend the live QuickJS WASI host boundary with `path_unlink_file`. The engine
forwards the Preview 1 import to the live provider when present; the read-only
virtual-WASI fallback remains unsupported. `wanix-qjs` implements the provider
method by delegating to `wanix_wasi::WasiCtx`.

## Consequences

Guest JavaScript can use `qjs:os.remove(...)` to remove files from the Wanix
namespace in native `wanix-rust qjs` demos. Host-directory mounts respect the
same explicit-root policy as other `LocalFs` operations.

This does not add directory creation/removal, rename, timestamp mutation, or fd
flag mutation. Those remain separate decisions because each has different
namespace and host-resource semantics.
