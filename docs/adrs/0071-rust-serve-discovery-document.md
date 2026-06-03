# ADR 0071: Rust Serve Discovery Document

## Context

Rust `wanix-rust serve` now combines static HTTP assets with a direct binary 9P
WebSocket export at `/.well-known/export9p`. Browser, v86, and future VS Code
clients still need a stable way to discover that route without duplicating
server-specific path knowledge in each frontend.

The endpoint should describe Rust serve's current native route contract without
pretending that unimplemented Go-era surfaces, such as the Ethernet/vnet bridge,
are available.

## Decision

Expose `GET /.well-known/wanix.json` from Rust `serve` as a JSON discovery
document. The document includes:

- the Rust runtime identifier;
- the direct binary 9P WebSocket URL for `/.well-known/export9p`;
- the 9P transport and protocol names;
- the reserved Ethernet WebSocket URL with status `not-implemented`;
- the optional `--bundle` hint.

The discovery response uses the HTTP `Host` header when it is safe, falling back
to the listener address when no usable host is present. It remains part of the
reserved `/.well-known` route surface and is not served from static files.

## Consequences

Browser/v86/VS Code experiments can now discover the Rust serve filesystem
export from one well-known HTTP request before opening a WebSocket. This moves
the next demo step toward composition around the Rust listener instead of
hard-coded client assumptions.

The discovery document intentionally advertises direct WebSocket 9P, not the Go
muxed TCP export shape. If a future compatibility bridge adds muxed exports,
that should be represented as an additional route rather than changing the
meaning of the existing direct 9P endpoint.
