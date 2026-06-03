# ADR 0076: 9P Lock Probe Compatibility

## Status

Accepted

## Context

Rust Wanix is moving toward real browser/v86, Linux, and editor clients over
the native 9P exports. Those clients can issue 9P2000.L advisory-lock probes
through `Tlock` and `Tgetlock`, especially from shells, package tools, language
servers, or editor workflows that call POSIX `fcntl` locking APIs.

The Go p9kit oracle treats lock acquisition as a successful compatibility stub
instead of maintaining a cross-client lock manager.

## Decision

Add dependency-free 9P2000.L `Tlock`/`Rlock` and `Tgetlock`/`Rgetlock` codecs in
`wanix-protocol`, and handle them in `wanix-9p` as compatibility probes:

- `Tlock` validates that the fid exists and replies with `Rlock` status `OK`.
- `Tgetlock` validates that the fid exists and replies with an `Rgetlock`
  payload whose lock type is `UNLOCK`, meaning no conflicting lock is present.
- Unknown fids return `EBADF`.

This does not add persistent advisory-lock state, blocking lock behavior, host
file locking, or cross-client lock conflict detection.

## Consequences

Linux/v86 and editor-style clients can run common lock probes against Rust
Wanix 9P exports without receiving `EOPNOTSUPP`. This improves mounted workflow
compatibility while keeping real lock-manager semantics as a future explicit
decision.

Future cycles can widen this into host-backed or Wanix-owned advisory-lock state
if a client needs actual conflict behavior rather than compatibility success.
