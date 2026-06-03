# ADR 0062: Sync 9P Stream Transport Loop

## Status

Accepted.

## Context

After the Rust 9P server could attach, walk, open, read, write, clunk, and list
directories, the next serve/v86/VS Code blocker was not another JavaScript
bridge. The server needed a native byte-stream boundary that can sit under Unix
sockets, TCP sockets, process pipes, or tests without choosing one deployment
surface too early.

## Decision

`wanix-9p` exposes a synchronous stream loop on `P9Server`:

```rust
server.serve_stream(reader, writer)
```

The loop reads bytes, uses `wanix-protocol::P9FrameBuffer` to split complete
frames, dispatches each decoded frame through `P9Server::handle_frame`, encodes
the response frame, and writes it to the supplied writer. Filesystem failures
remain normal `Rlerror` response frames. I/O failures, frame-splitting failures,
response-encoding failures, malformed typed requests, and EOF with a partial
frame become transport errors.

## Consequences

This gives future native listeners a small reusable core while keeping socket,
auth, concurrency, and CLI policy out of the filesystem-backed server. The
transport is intentionally synchronous for now because the current server and
filesystem traits are synchronous.
