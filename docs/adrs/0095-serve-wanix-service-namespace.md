# ADR 0095: Serve Wanix Service Namespace

## Status

Accepted

## Context

The Rust-served VS Code workbench can now populate Explorer through direct 9P,
but the 9P root was still only a host directory. That kept the browser path from
reaching Wanix service files such as `#task` and `#term`, even though the native
CLI already has Rust task and terminal devices.

Terminal data files are queue-like device files, not regular seekable files.
The existing 9P server always sought before `Tread` and `Twrite`, and the
workbench direct 9P client treated readable and writable streams as whole-file
helpers. Those choices were fine for host files but not for service devices.

## Decision

Add an explicit `wanix-rust serve --wanix-services` option. In this mode the 9P
export root is a Wanix namespace instead of a bare `LocalFs`:

- the served host directory is bound at `/`;
- a shared `TaskTable` service is bound at `#task`;
- a `TermDevice` service is bound at `#term`; and
- the first exposed driver is `noop`, so service allocation can be tested
  without implying that browser-started QuickJS shells are ready.

The discovery document advertises these service roots as:

```json
"services": { "task": "#task", "term": "#term", "drivers": ["noop"] }
```

When services are absent, discovery reports `"services": null`.

The generated `workbench-fs9p` launcher passes service config to the extension
only when discovery advertises it. The direct 9P client can now write existing
service files by opening them before falling back to create, and it treats
`#term/...` readable/writable handles as live streams. The 9P server preserves
offset semantics for seekable files but skips forced seeks for non-seekable
open fids, allowing queue-like device files to work over 9P.

## Consequences

The Rust serve/workbench path can now prove that browser-side clients can reach
Wanix service files over direct 9P. A test covers reading `#term/new` and
`#task/new/noop` through the same 9P server root used by `serve`.

This was not yet a complete interactive QuickJS shell in VS Code. ADR 0096
adds the next visible terminal slice: a discovery-advertised terminal/session
WebSocket route owned by Rust `serve`, with the session host reusing the native
`qjs-shell` terminal setup.

Because `--wanix-services` exposes service controls against a host-backed root,
auth, origin, and localhost/default-bind policy must be decided before treating
it as a production remote endpoint.
