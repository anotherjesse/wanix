# ADR 0059: Basic 9P Operation Codecs

## Status

Accepted

## Context

ADR 0058 created the dependency-free 9P framing boundary, but a future
Wanix-backed 9P server still needs typed request and response payloads before
it can focus on fid lifetime, namespace walking, open-file state, and error
mapping.

The existing Go/v86 path first depends on a small 9P2000.L operation set:
version negotiation, attach, walk, Linux open, read, write, clunk, QID payloads,
and Linux errno replies.

## Decision

Extend `wanix-protocol` with typed codecs for the first server-facing
9P2000.L operations:

- `Rlerror`
- `Tattach` and `Rattach`
- `Twalk` and `Rwalk`
- `Tlopen` and `Rlopen`
- `Tread` and `Rread`
- `Twrite` and `Rwrite`
- `Tclunk` and `Rclunk`

The crate still owns only wire shape. It preserves unknown frame types and does
not decide Wanix filesystem policy, authorization, fid tables, directory entry
encoding, server scheduling, or transport behavior.

## Consequences

The Rust port can now build a small 9P server around typed payloads instead of
hand-decoding byte fields in the server implementation. This is a direct step
toward native serve/v86 integration, while keeping the policy-bearing work in a
future server layer.

The next useful 9P cycle is to introduce a Wanix namespace-backed server that
handles version, attach, walk, open, read, write, and clunk over these codecs,
starting with in-process frame tests before wiring native transports.
