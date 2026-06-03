# ADR 0078: 9P Setattr Permissions

## Status

Accepted

## Context

Rust Wanix 9P exports are moving toward Linux/v86, browser, and editor clients
that operate on mounted filesystems. Those clients commonly use 9P2000.L
`Tsetattr` permissions to implement `chmod`, editor save flows, executable
script setup, and package-tool metadata updates.

ADR 0075 added `Tsetattr` size and timestamp support while leaving permissions
explicitly unsupported until the filesystem contract could represent permission
mutation.

## Decision

Add a Wanix filesystem permission mutation operation and route 9P
`Tsetattr(PERMISSIONS)` through it:

- `wanix-fs::FileSystem::set_permissions(path, permissions)` accepts
  Unix-style permission bits and lets implementations preserve their file type.
- `MemFs` updates its stored mode permission bits.
- `LocalFs` applies host permissions inside the existing rooted path checks.
- `Namespace` routes permission changes through normal bind resolution and
  rejects synthetic namespace directories as unsupported.
- `wanix-9p` treats `P9_SETATTR_PERMISSIONS` as supported, while `UID`, `GID`,
  and `CTIME` remain unsupported.

## Consequences

Mounted Linux/v86 and editor-style clients can now chmod files through Rust
Wanix 9P exports. This narrows the compatibility gap with the Go p9kit oracle
without adding ownership mutation or virtual uid/gid state.

Future work can add richer metadata operations such as uid/gid virtualization,
hard links, mknod, or xattrs when a client workflow proves the need.
