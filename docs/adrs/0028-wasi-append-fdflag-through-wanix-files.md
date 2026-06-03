# ADR 0028: WASI Append Fdflag Through Wanix Files

## Status

Accepted.

## Context

QuickJS exposes append mode through both `qjs:os.open(path, os.O_APPEND)` and
`qjs:std.open(path, "a")`. After create, remove, mkdir, rmdir, and rename,
append is the next ordinary file workflow needed by native qjs demos.

WASI Preview 1 represents append as an fdflag on `path_open`. Wanix already
owns the opened fd and shared file handle through `wanix-wasi`, so append can be
implemented without adding new filesystem metadata.

## Decision

Accept Preview 1 `FDFLAGS_APPEND` on file `path_open` requests when write
rights are present. Store append as part of the shared `WasiFile` access policy
and seek to end of file immediately before every write. This makes append
semantics hold even if guest code seeks the fd elsewhere before writing.

Report open-time append through `WasiFdStat` and the QuickJS engine fdstat
encoder. Continue to reject directory append and non-append fdflags such as
dsync, nonblock, rsync, and sync.

Leave `fd_fdstat_set_flags` unsupported for now. Runtime mutation of fd flags is
a separate fd-state decision and is not needed for QuickJS open-time append
workflows.

## Consequences

JavaScript running as a Wanix `qjs` task can append to Wanix namespace files via
`qjs:os.open(..., os.O_APPEND)` and `qjs:std.open(..., "a")`.

Append is enforced at the shared WASI file handle, so mirrored task fd reads and
writes see the same file content. Full mutable fd flag support remains queued.
