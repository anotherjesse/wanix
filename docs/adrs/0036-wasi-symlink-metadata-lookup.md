# ADR 0036: WASI Symlink Metadata Lookup

## Status

Accepted

## Context

WASI Preview 1 `path_filestat_get` carries lookup flags. In particular,
`LOOKUP_SYMLINK_FOLLOW` decides whether the final symbolic link component is
followed. The QuickJS engine already accepted and forwarded those flags to live
WASI providers, but the Wanix adapter discarded them and `wanix-wasi` exposed
only a flagless path stat operation.

`LocalFs` also canonicalized the whole host path before producing metadata.
That preserved host-mount escape protection for ordinary stat/open behavior,
but it made `lstat`-style metadata impossible because canonicalizing the whole
path follows the final symlink.

## Decision

Add a `wanix-fs` metadata lookup mode that distinguishes follow and no-follow
metadata for the final symbolic link component. Ordinary `FileSystem::metadata`
keeps the compatibility behavior of following final symlinks. Lookup-aware
metadata is an explicit opt-in path.

`LocalFs` implements no-follow metadata by canonicalizing only the parent
directory, checking that parent remains within the mounted root, and then
calling `symlink_metadata` on the raw final path. This allows metadata for final
symlinks, including broken or root-escaping links, while still rejecting
intermediate symlink traversal that escapes the mount.

`wanix-vfs` forwards the lookup mode through bind resolution. `wanix-wasi`
parses Preview 1 lookup flags for `path_filestat_get`: `flags = 0` reports the
final symlink itself, and `LOOKUP_SYMLINK_FOLLOW` reports the target. Unsupported
lookup bits remain `NOTCAPABLE`.

`wanix-qjs` passes the engine's live-WASI lookup flags into `wanix-wasi` instead
of discarding them. The bundled QuickJS fixture currently exposes `qjs:os.stat`
but not `qjs:os.lstat`, so this cycle proves the behavior at the LocalFs,
namespace, WASI, and QuickJS live-provider adapter layers.

## Consequences

Wanix now owns Preview 1 symlink metadata semantics across live WASI providers
without changing `path_open` or `read_dir` behavior. Existing Wanix callers that
use ordinary metadata continue to see followed metadata, while WASI path stat
can distinguish `stat` from `lstat`.

This does not add symlink creation, `path_readlink`, `path_symlink`,
QuickJS-level `os.lstat`, or broader path-open symlink policy.
