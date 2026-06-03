# ADR 0069: Serve Well-Known Routing

## Status

Accepted.

## Context

Rust `wanix-rust serve` can now serve static assets and accept binary 9P
WebSocket sessions. The Go `wanix serve` path reserves `/.well-known/*` for
protocol endpoints, including `/.well-known/export9p` and
`/.well-known/ethernet`.

The first Rust serve implementation accepted any WebSocket upgrade as 9P and
would serve static files under `.well-known` if they existed. That made the
route surface too loose for browser/v86 integration: a future qemu Ethernet
client could accidentally connect to a 9P server, and protocol routes could be
shadowed by static files.

## Decision

Make Rust `serve` path-aware:

- WebSocket upgrades to `/` and ordinary non-well-known paths continue to use
  the direct binary 9P WebSocket handler.
- WebSocket upgrades to `/.well-known/export9p` also use the direct binary 9P
  handler, giving browser clients a stable named export URL.
- `/.well-known/export9p` is not yet the Go muxed TCP-export bridge; it is the
  Rust direct 9P frame endpoint at a compatible discovery path.
- WebSocket or HTTP requests for `/.well-known/ethernet` return an explicit
  `501 Not Implemented` until the qemu/vnet bridge is implemented.
- HTTP requests under `/.well-known` are handled as reserved protocol routes,
  not as static files.

## Consequences

Rust serve now exposes a clearer browser-facing route contract while preserving
the existing direct 9P WebSocket proof. This reduces accidental coupling between
static assets and protocol endpoints, and leaves the qemu/v86 Ethernet bridge as
an intentional future cycle instead of a misleading 9P alias.

Future work should decide whether `/.well-known/export9p` must grow the Go
muxed TCP-export behavior or whether direct binary 9P over WebSocket is the
Rust-native browser contract.
