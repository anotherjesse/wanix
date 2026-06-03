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
```

The CLI binds each host root at the requested non-root Wanix guest path. Guest
path `.` is intentionally rejected for this demo surface so the synthetic
`main.js` script loader remains unambiguous and mounted host directories cannot
accidentally replace the CLI's task root.

## Consequences

QuickJS/WASI tasks can now demonstrate durable reads and writes against a real
host directory while still going through Wanix namespace resolution, task fd
state, and Wanix-owned WASI adapters.

This is a native CLI composition feature, not a change to core task identity or
QuickJS engine policy. Browser deployments and future persisted namespace
formats must choose their own host-resource attachment policy instead of
assuming local host paths are always available.
