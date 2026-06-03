# ADR 0017: Explicit Host Directory Mounts for Native qjs Demos

## Status

Accepted

## Context

The native Rust CLI can now run QuickJS/WASI tasks outside Chrome with a Wanix
namespace, task service files, stdio fds, and snapshot/restore demos. Until now
the CLI copied the selected script directory into an in-memory Wanix namespace,
which proved Wanix-owned semantics but left no simple way for a demo task to
read or write real host files through the namespace.

Wanix still must not delegate process or WASI semantics to host WASI. Host files
should appear only when the native composition layer explicitly grants them.

## Decision

Add `wanix_fs::LocalFs`, a `FileSystem` implementation rooted at one canonical
host directory. It maps normalized Wanix relative paths under that root,
supports ordinary file open/read/write/seek/tell/metadata/readdir behavior, and
rejects paths that resolve outside the configured root.

Expose host access in the native CLI through explicit qjs mounts:

```sh
wanix-rust qjs --mount HOST=GUEST script.js
wanix-rust qjs-restore --mount HOST=GUEST before.js after.js
```

The CLI binds each host root at the requested non-root Wanix guest path. Guest
path `.` is intentionally rejected for this demo surface so the synthetic
`main.js` script loader remains unambiguous and mounted host directories cannot
accidentally replace the CLI's task root.

## Consequences

QuickJS/WASI tasks can now demonstrate durable reads and writes against a real
host directory while still going through Wanix namespace resolution, task fd
state, and Wanix-owned WASI adapters.

For snapshot/restore demos, the QuickJS VM image keeps guest memory while the
mounted host directory is live namespace state cloned onto the restored Wanix
task. This keeps host resources outside the snapshot bytes while making the
reattachment visible through host filesystem writes.

For task-spawn demos, child tasks allocated through `#task/new/qjs` clone the
parent Wanix namespace, so explicit host mounts can provide both child program
source and child-visible storage without giving QuickJS its own process or host
filesystem policy.

This is a native CLI composition feature, not a change to core task identity or
QuickJS engine policy. Browser deployments and future persisted namespace
formats must choose their own host-resource attachment policy instead of
assuming local host paths are always available.
