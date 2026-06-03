# ADR 0027: WASI Rename Through Wanix Namespaces

## Status

Accepted.

## Context

QuickJS exposes file and directory rename through `qjs:os.rename(...)`, which
reaches WASI Preview 1 `path_rename`. Wanix needs that guest syscall to mutate
Wanix-owned namespace state, not host WASI state.

The Go implementation supports same-filesystem rename directly and can fall
back to copy/remove across filesystems. Copy/remove changes identity and failure
semantics, so it is not a good first WASI behavior for Wanix task namespaces.

ADR 0011 already keeps live WASI providers out of `QuickJsHostConfig`.
Rename therefore extends the existing runtime-owned `QuickJsWasiHost` provider
surface rather than adding live Wanix state to deterministic host config.

## Decision

Add `FileSystem::rename(old_path, new_path)` as a same-filesystem filesystem
operation. `MemFs` and `LocalFs` support file rename, file-over-file
replacement, directory subtree rename, and directory-over-empty-directory
replacement. Root rename is rejected. File-over-directory, directory-over-file,
directory-over-non-empty-directory, missing source, and missing target parent
are pinned as explicit errors.

Add `Namespace::rename(old_path, new_path)` only when source and target resolve
to the same concrete backing filesystem. Cross-filesystem and cross-binding
copy/remove fallback is out of scope and currently reports unsupported.
Synthetic namespace directories cannot be renamed as concrete filesystem
objects.

Add WASI Preview 1 rights `PATH_RENAME_SOURCE` and `PATH_RENAME_TARGET`, and
include both in directory base and inheriting rights. `WasiCtx::path_rename`
checks source and target directory rights independently before delegating to the
namespace.

Extend `QuickJsWasiHost` with `path_rename`. The engine crate copies both guest
paths from Wasmtime memory and forwards the call to the live provider. Wanix
policy remains in `wanix-wasi`, `wanix-vfs`, and `wanix-fs`; the engine remains
QuickJS/Wasmtime plumbing.

## Consequences

JavaScript running as a Wanix `qjs` task can now rename files and directory
subtrees through `qjs:os.rename(...)`, including in the native CLI demo.

Wanix does not yet support cross-filesystem rename. If Wanix later needs Go's
copy/remove fallback, it should be added as an explicit namespace policy with
its own error, partial-failure, and fd/snapshot implications.
