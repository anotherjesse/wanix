# ADR 0097: Serve qjs Task Service Driver

## Status

Accepted

## Context

ADR 0096 added a dedicated `/.well-known/qjs-shell` WebSocket route so the
workbench can drive an interactive QuickJS shell through terminal bytes. That
route is useful, but it should not become a separate process model beside
Wanix `#task`.

The Rust `#task` service already supports allocation through `#task/new/*`,
task metadata files, fd binding, and `ctl start`. Rust `serve --wanix-services`
previously exported only a `noop` driver, so browser or 9P clients could prove
service reachability but could not start real QuickJS/WASI tasks through the
service namespace.

## Decision

When `wanix-rust serve --wanix-services` is enabled:

- register both `noop` and `qjs` in the service `TaskTable`;
- allocate a service-root `noop` task with the served host root and `#term`
  bound into its namespace;
- export `#task` as that service-root task's current task view; and
- advertise `["noop","qjs"]` in discovery services.

The qjs driver uses the same bundled QuickJS runner cache as the CLI. The
dedicated qjs-shell WebSocket route remains separate because it owns a live
terminal session and pumps ready-IO between browser input frames.

## Consequences

External clients using the Rust direct 9P export can now allocate and start a
real QuickJS task through `#task/new/qjs`, set `cmd`/`dir`/`env`, bind fds, and
observe exit state. Child tasks inherit the served namespace and `#term` from
the service-root task instead of starting in an empty namespace.

This is still a synchronous one-shot task service. A long-running `ctl start`
blocks that connection worker until the driver returns. Future task/session
supervision should add async lifecycle, cancellation, signal policy, and richer
terminal attachment rather than overloading this first service proof.
