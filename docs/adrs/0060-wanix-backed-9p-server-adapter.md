# ADR 0060: Wanix-Backed 9P Server Adapter

## Status

Accepted

## Context

Rust Wanix now has dependency-free 9P framing and typed basic operation codecs
in `wanix-protocol`. The next serve/v86 milestone needs a server layer that
maps those protocol frames onto Wanix filesystem semantics without mixing that
policy into the wire crate.

The Go p9kit path keeps fid state, performs attach/walk/open/read/write/clunk
operations, maps filesystem failures into Linux errno replies, and leaves
transport-specific framing outside the filesystem implementation.

## Decision

Add a `wanix-9p` crate that depends on `wanix-fs` and `wanix-protocol`.

The initial server is in-process and handles the first server-facing operation
set:

- `Tversion`
- `Tattach`
- `Twalk`
- `Tlopen`
- `Tread`
- `Twrite`
- `Tclunk`

Filesystem failures are returned as `Rlerror` with Linux errno values. Malformed
protocol payloads remain Rust errors so a transport can decide whether to close
the connection.

## Consequences

Rust Wanix now has a tested 9P server core over `wanix-fs`, which moves native
serve/v86 integration from pure wire helpers toward an actual filesystem export
path.

This server does not yet implement directory reads, create/remove/rename,
stat/getattr/setattr, symlinks, transport loops, concurrent request handling,
or task/namespace-specific authorization policy.
