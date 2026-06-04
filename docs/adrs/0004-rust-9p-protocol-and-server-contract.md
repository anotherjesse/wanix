# ADR 0004: Rust 9P Protocol and Server Contract

## Status

Accepted

## Context

9P is the main protocol path for Linux, v86, editor, and browser filesystem
clients to browse and mutate a Wanix namespace from outside the runtime. The
Rust port needs a protocol stack that keeps wire codecs, server fid state,
filesystem semantics, and transports separate enough to evolve safely.

This ADR covers the Rust-owned 9P boundary. Per-operation coverage belongs in
protocol/server tests and current-state docs unless a new operation changes the
public filesystem or trust contract.

## Decision

`wanix-protocol` owns dependency-free 9P frame splitting, tag extraction,
version negotiation, and typed operation codecs. It covers the server-facing
9P2000.L surface and selected compatibility codecs only when clients require
them.

`wanix-9p` owns the server state that maps fids to Wanix filesystem objects and
open-file state. It translates filesystem results into 9P replies and Linux-ish
errno errors while leaving listener, socket, stdio, and browser policy to
adapters.

The supported contract includes:

- explicit version negotiation and rejection of unsupported protocol versions;
- fid lifecycle, attach, walk, open, create, read, write, clunk, and error
  mapping;
- directory iteration with opaque cookies;
- metadata, statfs, permissions, size, timestamp, link, rename, remove, mkdir,
  and append behavior where the backing Wanix filesystem supports it;
- compatibility probes for client feature detection; and
- selected protocol extensions needed by Linux, v86, editor, or browser clients.

Stdio, TCP, WebSocket, and `serve` transports are adapters over the same server
contract. They must preserve binary frame boundaries and keep diagnostic/status
output out of binary response streams.

Unsupported features should return deliberate protocol errors until Wanix has a
backing contract. Rust Wanix should not fake auth, special-file, extended
attribute, ownership, inode-link, or device semantics beyond what the Wanix
filesystem contract can actually provide.

## Consequences

External 9P clients can mount or browse a Wanix namespace through native,
browser, v86, and editor paths without each transport inventing filesystem
semantics. Operation-by-operation coverage belongs in codec/server tests and
current-state docs, not in one ADR per operation.

Future 9P work should update or add ADRs only when it changes the protocol
contract, authentication/trust boundary, transport multiplexing model, or
backing Wanix filesystem semantics.
