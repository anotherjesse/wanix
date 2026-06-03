# ADR 0020: Mirror qjs WASI File Fds Into Wanix Tasks

## Status

Accepted.

## Context

ADR 0009 makes QuickJS an execution engine inside a Wanix task, not an owner of
separate process identity. Wanix owns task fds through `#task/<id>/fd/*`, while
QuickJS libc and `qjs:os` open namespace files through Wanix-backed WASI.

Before this ADR, dynamic fds opened by WASI `path_open` lived only inside
`wanix_wasi::WasiCtx`. They could read and write Wanix namespace files, but the
same fd number was not visible through `#task/self/fd/<n>`. That made
file-oriented task controllers see a different fd model from guest JavaScript.

## Decision

Keep `wanix-wasi` independent of `wanix-task`, but let `WasiCtx` accept an
optional dynamic-fd observer. For qjs task runtimes, install a task observer that:

- mirrors each dynamic regular-file WASI fd into the Wanix task fd table at the
  same numeric fd;
- stores a clone of the same shared `WasiFile` handle, preserving file offset
  and read/write access;
- removes the mirrored task fd when WASI `fd_close` closes the dynamic fd, and
  also when the owning `WasiCtx` drops with guest fds still open;
- treats `fd_close` as transactional with respect to the observer: if the
  mirrored Wanix fd cannot be closed, the WASI fd remains open rather than
  silently leaving the two fd views out of sync.

Standard fds and preopens keep their existing fixed attachment behavior.
Directory fds remain WASI-internal for now because Wanix task fd proxy files
currently model byte-file handles.

## Consequences

Guest JavaScript can open a namespace file with `qjs:os.open(...)`, then address
the same live fd through `#task/self/fd/<n>` or bind it into another task through
`#task/<child>/ctl`.

If guest JavaScript exits without closing a dynamic file fd, dropping the qjs
task runtime cleans the mirrored Wanix fd instead of leaking it in `#task`.

Snapshots still reject open dynamic WASI fds. Mirroring makes the fd visible to
Wanix, but it does not serialize the live host file handle into the VM image.
