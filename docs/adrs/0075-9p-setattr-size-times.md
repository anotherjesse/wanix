# ADR 0075: 9P Setattr Size and Timestamps

## Context

Updated by [ADR 0078](0078-9p-setattr-permissions.md), which adds
`Tsetattr(PERMISSIONS)` support for chmod-compatible workflows.

Rust Wanix 9P exports are moving toward browser/v86, Linux, and editor clients.
Those clients can use 9P2000.L `Tsetattr` for file truncation, timestamp
updates, permission changes, ownership changes, and metadata-change timestamps.

Wanix already has filesystem contracts for open-file resizing and path
access/modification time updates. At the time of this decision, it did not yet
have filesystem contracts for chmod, chown, or explicit ctime mutation.

## Decision

Add `Tsetattr`/`Rsetattr` codecs in `wanix-protocol`, and handle the Wanix-owned
subset in `wanix-9p`:

- `P9_SETATTR_SIZE` opens the fid path for writing and calls `File::set_len`.
- `P9_SETATTR_ATIME` and `P9_SETATTR_MTIME` call `FileSystem::set_times`,
  preserving any timestamp not requested by the client.
- explicit timestamp fields are validated before mutation; invalid timestamps
  return `EINVAL` without resizing the file first.
- system-time timestamp requests use the native server clock.
- permission, uid, gid, and ctime updates return `EOPNOTSUPP`; permissions are
  widened later by ADR 0078.
- unknown valid-mask bits return `EINVAL`.

Unsupported mask bits are rejected before applying supported updates so mixed
requests do not produce partial success for attributes Wanix cannot yet model.

## Consequences

External 9P clients can now truncate files and update access/modification times
through the Rust server, including host-backed `LocalFs` exports served by the
native TCP and browser WebSocket listeners.

This kept chmod/chown/ctime honest at the time: clients received an explicit
unsupported error until Wanix grew those filesystem contracts. ADR 0078 later
adds mode mutation to `wanix-fs` and widens the supported `Tsetattr` mask for
permissions, while ownership and explicit ctime changes remain future work.
