# ADR 0080: 9P Virtual Ownership

## Status

Accepted.

## Context

Rust `serve --bundle direct-v86` now gives browser/v86 clients a route to the
Rust 9P WebSocket export. That makes Linux-style mounted-client behavior more
important: clients can issue `Tsetattr` requests for uid/gid changes and expect
later `Tgetattr` responses to report the result.

The Rust `wanix-fs` contract does not yet own persistent file ownership. The Go
serve path uses a virtual attribute store backed by host xattrs for uid/gid, but
copying that into `wanix-fs` now would force a persistence and host-policy
decision before the Rust 9P path has proved the rest of the mounted workflow.

## Decision

Store 9P uid/gid ownership as session-virtual metadata inside `wanix-9p`.

- `Tsetattr(UID/GID)` records owner values for the fid path after validating
  the fid and backing path.
- `Tgetattr` reports the recorded values, defaulting to uid/gid `0` when no
  virtual owner exists.
- `Tlcreate`, `Tmkdir`, and `Tsymlink` preserve their requested gid as virtual
  owner metadata for the created path.
- `Trenameat` moves virtual ownership metadata with the renamed path.
- `Tunlinkat` clears virtual ownership metadata for the removed path.
- `wanix-fs` does not gain a chown or xattr API in this decision.

## Consequences

Mounted Linux/v86 and editor-style clients can observe chown-style ownership
changes during a Rust 9P server session. This narrows a real compatibility gap
without pretending that host-backed ownership persistence is solved.

The metadata is path-keyed and in-memory. It is not persisted across process
restart, snapshot, or a future exported-state boundary. If a later workflow
needs durable ownership, Rust should add an explicit ownership/xattr store
decision rather than silently changing the meaning of this session-virtual
metadata.
