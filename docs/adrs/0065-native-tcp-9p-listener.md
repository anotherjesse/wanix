# ADR 0065: Native TCP 9P Listener

## Status

Accepted.

## Context

Rust Wanix now has a filesystem-backed 9P server, a synchronous request/response
stream loop, a stdio bridge, and metadata replies for clients that stat fids
before opening them. That makes the protocol path testable, but external
serve/v86/VS Code experiments still need a native socket boundary.

The listener should not pull connection policy into `wanix-9p`. The server crate
owns fid state and maps 9P frames onto `wanix-fs`; the CLI/composition layer owns
native deployment choices such as TCP binding, logging, and whether a command
serves one connection or stays alive.

## Decision

Add `wanix-rust p9-listen --root DIR --addr HOST:PORT [--once]`:

- the command exports a `LocalFs` root through `P9Server`;
- each TCP connection gets a fresh `P9Server`, so fid state is connection-local;
- connections are accepted serially for now;
- `--once` serves one connection and exits, giving tests and scripted demos a
  deterministic lifecycle;
- stdout is unused, and stderr carries listener/transport status.

The existing `p9-stdio` bridge remains useful for process-spawn integrations and
binary fixtures. The TCP listener is the first native socket export, not a
browser WebSocket bridge or a final auth policy.

## Consequences

External tools can now connect to Rust Wanix over TCP and browse a host-root
filesystem export using the same 9P server core that the stdio bridge exercises.
This moves serve/v86 integration from an internal stream harness to a reachable
native endpoint.

Future cycles still need mutation operations, an explicit auth/exposure policy,
and a WebSocket or other browser-facing transport if v86 cannot use raw TCP in a
given deployment.
