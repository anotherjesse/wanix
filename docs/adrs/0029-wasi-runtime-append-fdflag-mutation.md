# ADR 0029: WASI Runtime Append Fdflag Mutation

## Status

Accepted.

## Context

ADR 0028 accepted open-time append fdflags and deliberately left
`fd_fdstat_set_flags` unsupported. QuickJS also reaches append through
`std.fdopen(fd, "a")`, which wraps an existing fd and asks WASI libc to mutate
that fd's flags. Without runtime mutation, writes through the wrapped fd use the
current offset and can overwrite the start of a Wanix file.

Dynamic qjs task fds are also mirrored into the Wanix task fd table. Any runtime
fdflag mutation must therefore update shared fd state, not only the fdstat bits
stored in `WasiCtx`.

## Decision

Support Preview 1 `fd_fdstat_set_flags` for Wanix-backed regular file fds with
exactly two flag states: `0` and `FDFLAGS_APPEND`. Enabling append requires
write capability. Clearing flags returns the fd to ordinary offset-based writes.
Unknown fdflag bits remain `NOTCAPABLE`.

Store the append bit in the shared `WasiFile` access policy so cloned task-fd
mirrors observe the same write behavior as the live WASI fd. Continue to keep
the QuickJS engine responsible only for copying the syscall arguments and
forwarding them to the live provider.

## Consequences

JavaScript can append through `std.fdopen(fd, "a")` after opening a Wanix file
without `O_APPEND`, and writes through mirrored `#task/.../fd/<n>` handles stay
consistent with the mutated fd state.

The read-only virtual engine filesystem remains immutable and does not grow
runtime fdflag support. Non-append fdflags and richer mutable fd state remain
queued until Wanix needs them.
