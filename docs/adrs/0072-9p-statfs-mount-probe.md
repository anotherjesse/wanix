# ADR 0072: 9P Statfs Mount Probe

## Context

The Rust 9P server is becoming the filesystem boundary for native serve, v86,
and future editor integrations. Linux/v86 clients commonly probe filesystem
statistics with `Tstatfs`; returning `EOPNOTSUPP` there can stop or degrade a
mount before ordinary file browsing gets a chance to prove the export.

The current `wanix-fs` traits do not yet expose filesystem capacity, inode
counts, or host filesystem ids.

## Decision

Handle 9P2000.L `Tstatfs` in `wanix-protocol` and `wanix-9p`. The server
requires the supplied fid to exist and still resolve, then returns a synthetic
Wanix filesystem stat:

- type magic `0x01021997`;
- block size matching the Rust 9P server's default block size;
- stable nonzero filesystem id;
- maximum name length `255`;
- zero capacity and inode counts until `wanix-fs` grows a real statfs contract.

Unknown fids return `EBADF` like other fid-scoped operations.

## Consequences

External 9P clients can now complete a common mount/browse probe against the
Rust server instead of seeing an unsupported operation. The response is
deliberately conservative: it unblocks clients without pretending that Wanix
has authoritative capacity accounting yet.

When `wanix-fs` gains capacity/statfs metadata, this synthetic response should
be replaced or layered with real backing-filesystem values while preserving the
same `Tstatfs` wire contract.
