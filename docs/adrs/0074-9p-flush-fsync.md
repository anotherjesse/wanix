# ADR 0074: 9P Flush and Fsync Compatibility

## Context

Rust Wanix is moving toward real browser/v86, Linux, and editor clients over
the Rust 9P exports. Those clients can send `Tflush` while canceling an older
request and `Tfsync` after writes or mount-level sync probes.

The current Rust 9P server handles requests synchronously and does not keep a
pending-request table. The `wanix-fs::File` trait also does not yet expose a
durable sync operation, so the server cannot promise host-backed storage
durability without first adding a real filesystem contract.

## Decision

Add dependency-free 9P2000.L `Tflush`/`Rflush` and `Tfsync`/`Rfsync` codecs in
`wanix-protocol`, and handle them in `wanix-9p`:

- `Tflush` decodes the old tag and replies with `Rflush`. With the synchronous
  server core there is no queued in-flight request to cancel.
- `Tfsync` validates that the fid exists, then replies with `Rfsync`.
- `Tfsync` for an unknown fid returns `EBADF`.

This makes the operation explicit instead of falling through to unsupported
operation errors, while preserving the current Wanix filesystem trait boundary.

## Consequences

External 9P clients have a better chance of completing common sync and
cancelation paths against Rust Wanix exports. This advances the v86/Linux and
VS Code integration path without reshaping the filesystem layer prematurely.

`Tfsync` is a compatibility no-op for now. If Wanix needs durable host sync
semantics, add an explicit filesystem/file sync trait method and update this
handler to call it for implementations that support it.
