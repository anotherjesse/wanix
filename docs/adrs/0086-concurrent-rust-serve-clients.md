# ADR 0086: Concurrent Rust Serve Clients

## Status

Accepted.

## Context

Rust `serve` combines static HTTP, discovery, embedded direct-v86 assets, and a
direct 9P WebSocket export on one listener. The first implementation accepted
and handled each connection synchronously. That preserved simple tests, but it
meant a long-lived `/.well-known/export9p` WebSocket mount could monopolize the
listener and block follow-up discovery, asset, or editor requests.

The Go serve path uses `net/http`, so static requests and WebSocket sessions are
handled concurrently.

## Decision

Non-`--once` Rust `serve` accepts connections in a concurrent loop and handles
each HTTP or WebSocket connection on its own worker thread. Accepted streams are
restored to blocking mode before HTTP/WebSocket handling so platform-specific
listener nonblocking behavior does not leak into tungstenite or ordinary stream
reads.

`--once` remains a single-connection deterministic mode for tests and scripted
smokes.

## Consequences

A browser or VM can keep a direct 9P WebSocket open while other clients continue
to fetch `/.well-known/wanix.json`, embedded v86 assets, or ordinary static
files. This moves the Rust serve path from a handoff proof toward a real
mounted-client workflow for v86 and future VS Code/editor integrations.

The current implementation uses one native thread per connection. That is
acceptable for the local demo server; a future async or bounded-pool design can
replace it if serve grows into a higher-concurrency production surface.
