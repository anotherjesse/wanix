# ADR 0025: WASI Directory Create Through Wanix Namespaces

## Status

Accepted.

## Context

QuickJS/WASI tasks can create and remove regular files through Wanix-backed
WASI, but directory creation still stopped at the engine's unsupported
mutation fallback. That left ordinary JavaScript workflows unable to prepare
subdirectories before writing generated files, even though Wanix already owns
the task namespace, current directory, descriptor rights, and backing
filesystem routing.

Directory creation is a namespace mutation and a trust boundary. The engine
must not learn Wanix policy; it should only copy Preview 1 guest memory and
forward the call to a live provider when one is attached.

## Decision

Add exact one-directory creation to the shared filesystem contract with
`FileSystem::create_dir`. `MemFs` and `LocalFs` implement non-recursive
creation: the parent must already exist as a directory, the destination must
not already exist, and `LocalFs` validates the canonical parent stays inside
its configured host root.

`Namespace` routes `create_dir` through the highest-priority resolved bind
target. Synthetic namespace-only directories are visible as existing
directories but are not materialized by creation.

Add `WasiRights::PATH_CREATE_DIRECTORY` and expose
`WasiCtx::path_create_directory(dirfd, path)`. The operation resolves through
the same Wanix path rules as other WASI path calls and requires the directory
fd to carry `PATH_CREATE_DIRECTORY`.

Extend the live QuickJS WASI host boundary with `path_create_directory`. The
engine forwards the Preview 1 import to the live provider when present, while
the read-only virtual-WASI fallback remains unsupported. `wanix-qjs` delegates
the provider method to `wanix_wasi::WasiCtx`.

## Consequences

Guest JavaScript can use `qjs:os.mkdir(...)` in native `wanix-rust qjs` demos
to create directories in the Wanix namespace and then write files beneath
them.

This does not add directory removal, rename, timestamp mutation, or recursive
directory creation. Those remain separate decisions because they have distinct
namespace and host-resource semantics.
