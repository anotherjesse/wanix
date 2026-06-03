# ADR 0063: Stdio 9P CLI Bridge

## Status

Accepted.

## Context

The Rust 9P server can now handle encoded request/response streams, but an
external integration still needs a native process boundary. Choosing TCP,
Unix-domain sockets, browser WebSockets, auth, and long-lived concurrency in
one step would mix deployment policy with the filesystem-backed server core.

## Decision

`wanix-rust p9-stdio --root DIR` serves a host directory through `wanix-9p`
over process stdin/stdout:

- stdin is a binary 9P request stream;
- stdout is a binary 9P response stream;
- stderr is reserved for human-readable CLI or transport errors;
- the root is a `wanix-fs::LocalFs`, preserving the existing host-root escape
  protections.

The command exits `0` after clean EOF and exits `1` after a transport error.
Invalid CLI arguments remain ordinary usage errors.

## Consequences

This gives serve/v86/VS Code experiments a spawnable process bridge without
committing to socket policy yet. Future native listeners can reuse the same
`P9Server::serve_stream` path under a socket or WebSocket frontend.
