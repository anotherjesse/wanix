# ADR 0039: QuickJS WASI Truncate Stdlib Fixture

## Status

Accepted

## Context

ADR 0033 added Wanix-owned `fd_filestat_set_size` support across filesystem,
WASI, engine, and qjs adapter layers. That made file-size mutation available to
WASI guests, but ordinary JavaScript still had no `qjs:os` API that reached the
import because the bundled QuickJS fixture did not expose `truncate` or
`ftruncate`.

Wanix already owns the rights checks, fd table, namespace mapping, and file
resize semantics. The engine fixture only needs to expose a JavaScript-facing
QuickJS libc helper that calls the existing WASI libc functions.

## Decision

Rebuild the checked-in `wanix-qjs-engine` QuickJS fixture with two additional
`qjs:os` functions:

- `os.ftruncate(fd, size)` calls WASI libc `ftruncate`.
- `os.truncate(path, size)` calls WASI libc `truncate`.

Both functions return QuickJS libc errno-style integer results, matching
existing `qjs:os` mutation helpers. The resulting fixture imports
`fd_filestat_set_size`; Wanix policy remains in live WASI providers.

## Consequences

JavaScript running as a Wanix `qjs` task can now shrink and grow regular files
through `qjs:os.ftruncate` and `qjs:os.truncate`, with growth zero-filling
through the underlying Wanix filesystem handle. The proof spans the engine
fixture, the `wanix-qjs` task driver, and a native CLI demo.

Changing the fixture changes the module SHA-256 used by snapshot identity
validation. Older snapshots made with the prior fixture are expected to be
rejected by the exact-build snapshot contract.

This does not add a Preview 1 path-size syscall, engine-owned mutable virtual
file resize behavior, or broader path-open policy. Path truncation reaches
Wanix by letting WASI libc open the path and then call `fd_filestat_set_size`.
