# ADR 0096: Serve qjs-shell Terminal Route

## Status

Accepted

## Context

Rust Wanix already has a native `qjs-shell` CLI demo that runs the bundled
QuickJS shell as a Wanix task with `#term`-backed stdio. The Rust-served VS Code
workbench can also reach host files and Wanix service files over direct 9P, but
that path alone cannot drive an interactive shell because QuickJS ready-IO turns
must be pumped while browser terminal input arrives.

The browser shell path must not invent a second QuickJS process model. Wanix
owns task identity, cwd/env/cmd, fd state, terminal devices, and exit state;
QuickJS is the execution engine inside the `qjs` task driver.

## Decision

Add a `/.well-known/qjs-shell` WebSocket route to Rust `serve` when
`--wanix-services` is enabled. Discovery advertises the route as:

```json
{
  "protocol": "wanix-qjs-shell.v1",
  "mode": "raw-bytes",
  "status": "available"
}
```

The route reuses the same bundled shell source and Wanix task/terminal setup as
the native `qjs-shell` command:

- the served host root is bound into the task namespace at `/`;
- `#term` backs task stdio;
- `WANIX_QJS_SHELL_RAW=1` lets the guest shell own visible echo behavior;
- binary WebSocket messages are terminal input bytes;
- binary WebSocket responses are terminal output bytes; and
- text lifecycle messages report exit status.

The generated `workbench-fs9p` launcher passes the discovered WebSocket URL to
the workbench extension. The extension creates a VS Code pseudoterminal backed
by that raw-byte WebSocket when the user requests `?term=1`.

## Consequences

Rust `serve --bundle workbench-fs9p --wanix-services` now has a visible
interactive QuickJS-backed terminal path outside Chrome's original Wanix
runtime. Tests cover route discovery, HTTP reservation, the reusable shell
session object, and a real WebSocket transcript.

This remains a local dev/demo route. It is synchronous per terminal session,
does not yet expose arbitrary task driver selection, and needs explicit auth,
origin, cancellation, signal, resize, and lifecycle policy before becoming a
remote production endpoint. Longer term, the route should share more of the
task/session supervision machinery that will also serve qemu/v86 and VS Code
integration.
