# ADR 0064: 9P Getattr Metadata

## Status

Accepted.

## Context

External 9P clients commonly stat fids before opening or reading them. The Rust
9P path already supported attach, walk, open, read, write, readdir, clunk, a
sync stream loop, and a stdio CLI bridge, but clients still received
`EOPNOTSUPP` for `Tgetattr`.

The existing Go p9kit and v86 code use the 9P2000.L fixed `Rgetattr` payload:
`valid`, `qid`, POSIX mode, uid/gid, link/device/size/block fields, timestamps,
birth time, generation, and data version.

## Decision

`wanix-protocol` owns typed `Tgetattr` and `Rgetattr` codecs for the
9P2000.L field order. `wanix-9p` maps `Tgetattr` to `FileSystem::metadata` for
the fid path and replies with:

- the requested mask echoed as `valid`, matching Go p9kit behavior;
- deterministic Wanix QIDs;
- mode values with POSIX file-type bits added when the backing metadata only
  carries permission bits;
- size and Wanix metadata timestamps split into seconds/nanoseconds;
- default uid/gid/rdev/generation/data-version values until Wanix metadata grows
  richer ownership or device fields.

## Consequences

`wanix-rust p9-stdio --root DIR` can now answer client stat requests over its
binary stdio bridge. This unblocks more realistic serve/v86/VS Code experiments
before implementing mutation operations.
