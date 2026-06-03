# ADR 0013: Preview 1 File Rights Projection

## Status

Accepted

## Context

QuickJS-NG `qjs:std` file APIs are libc wrappers. For a read-only
`std.loadFile("input.txt")`, the bundled WASI libc opens the path with
`dirflags=LOOKUP_SYMLINK_FOLLOW`, no open/fd flags, and a broad rights mask
that includes regular-file read/seek/stat rights plus directory path rights.
For `std.writeFile("created.txt", ...)`, libc uses the same lookup flag,
`O_CREAT | O_TRUNC`, and a broad write/seek/stat rights mask.

Wanix previously rejected that request because `WasiCtx::path_open_preview1`
required every requested base right to be a regular-file right when the resolved
path was a regular file. That protected the trust boundary, but it also rejected
harmless directory rights that are not meaningful on the opened file descriptor.

The same shape appears on Wanix service files. QuickJS `qjs:os.open` asks for
seek/tell/stat rights even when opening write-only `#task/<id>/cmd`,
`#task/<id>/env`, `#task/<id>/dir`, or `#task/<id>/ctl` handles. Those service
files can enforce writes without supporting every requested libc file right.

## Decision

Keep rejecting unknown Preview 1 rights. When a `path_open` request resolves to
a file handle, project the requested base rights onto rights the opened handle
actually supports before reporting `fdstat`.

The effective file fd only grants rights Wanix can enforce: `FD_READ`,
`FD_WRITE`, `FD_SEEK`, `FD_TELL`, and `FD_FILESTAT_GET`. Requested directory
path rights are accepted only as part of the raw open request; they are not
granted to the opened file fd. Projection must preserve the requested read or
write mode, so a read open still requires effective `FD_READ` and a write open
still requires effective `FD_WRITE`.

Directory opens keep their existing stricter behavior: requested directory base
and inheriting rights must be directory-capable and parent-authorized.

## Consequences

Guest JavaScript can now read and write Wanix namespace files through QuickJS
libc APIs such as `std.loadFile(...)`, `std.writeFile(...)`, and
`os.open`/`os.read`/`os.write` without giving QuickJS its own process or
filesystem identity.

Guest JavaScript can also write Wanix task service files through `qjs:os`
without granting service handles fake seek/tell behavior. `std.writeFile(...)`
still remains inappropriate for service fields because it requests
create/truncate semantics that `#task` files do not have.

The engine crate remains Wanix-agnostic. It forwards guest ABI calls to a live
`QuickJsWasiHost`; the `wanix-qjs` adapter converts those calls into
`wanix_wasi::WasiCtx` operations backed by task namespaces and fds.

This is not a blanket Preview 1 compatibility pass. Append/nonblock/sync
fdflags, exclusive/directory-specific open modes, directory mutation calls, and
richer rights such as fd allocation remain explicit follow-ups until Wanix
defines their semantics.
