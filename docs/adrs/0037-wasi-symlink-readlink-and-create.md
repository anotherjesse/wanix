# ADR 0037: WASI Symlink Readlink and Create

## Status

Accepted

## Context

ADR 0036 added no-follow symlink metadata so live Wanix-backed WASI providers
could distinguish a final symlink from the target it points at. That left the
next symlink substrate gap: Preview 1 guests could not read a link target with
`path_readlink` or create a link with `path_symlink`.

Wanix owns filesystem, namespace, fd, and rights semantics for QuickJS tasks.
The QuickJS engine should only decode Preview 1 imports and forward them to the
live provider when one is attached.

## Decision

Add Wanix-owned `path_readlink` and `path_symlink` support across
`wanix-fs`, `wanix-vfs`, `wanix-wasi`, `wanix-qjs`, and the live
`QuickJsWasiHost` provider surface.

`path_readlink` resolves the path through the task namespace using the
`PATH_READLINK` right, reads the final symlink itself, and returns the link
target bytes. At the engine boundary, a short guest buffer succeeds and copies
only the prefix that fits, reporting the copied byte count without adding a NUL
terminator.

`path_symlink` resolves the new link path through the task namespace using the
`PATH_SYMLINK` right. The target operand is treated as link contents, not as a
Wanix path to normalize or authorize. Wanix rejects oversized or NUL-containing
targets, then passes the remaining bytes to the filesystem.

`LocalFs` implements both operations by canonicalizing only the parent directory
of the final component. This preserves the host-mount boundary for link creation
and final-link inspection, while still allowing broken, absolute, or
root-escaping link targets to exist as inert link contents. Opening or following
such targets remains subject to the existing mount-escape checks.

Without a live provider, the QuickJS engine keeps the previous unsupported-path
fallback and does not grow engine-owned mutable symlink policy.

## Consequences

Wanix now owns Preview 1 symlink read/create semantics for live WASI providers.
The behavior is proven at the engine WAT import layer, the `wanix-qjs` adapter,
the `wanix-wasi` context, VFS bind resolution, and LocalFs host mounts. ADR 0038
later makes this behavior directly visible to guest JavaScript through
`qjs:os.readlink`, `qjs:os.symlink`, and `qjs:os.lstat`.

This does not change `path_open` symlink following policy, add Windows symlink
creation support, or add snapshot serialization for open symlink-related fd
state.
