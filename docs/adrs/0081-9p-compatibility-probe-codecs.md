# ADR 0081: 9P Compatibility Probe Codecs

## Status

Accepted.

## Context

The Rust 9P export is now a direct dependency of the larger v86, Linux guest,
VS Code, and browser `serve` direction. Those clients can issue 9P2000.L
messages for auth negotiation, special-file creation, hard links, and extended
attributes even when a particular mounted workflow can tolerate those
operations being unsupported.

Previously these message types were anonymous unsupported frames in Rust. That
made traces harder to read and prevented the server from validating fids and
path shapes before returning a Linux-style errno.

## Decision

Add dependency-free typed codecs for these 9P2000.L probe messages in
`wanix-protocol`:

- `Tauth` and `Rauth`
- `Tmknod` and `Rmknod`
- `Tlink` and `Rlink`
- `Txattrwalk` and `Rxattrwalk`
- `Txattrcreate` and `Rxattrcreate`

Handle the request messages explicitly in `wanix-9p`:

- Unknown fids return `EBADF`.
- `Tauth` returns `ENOSYS` without binding the auth fid because Rust Wanix does
  not require a separate 9P auth phase yet.
- Valid fids with unsupported special-file, hard-link, or extended-attribute
  semantics return `EOPNOTSUPP`.
- `Tmknod` and `Tlink` validate the target basename through Wanix path
  normalization before reporting unsupported.
- `Txattrwalk` does not bind the requested new fid when xattrs are unsupported.

This does not add special files, hard-link mutation, xattr streams, or durable
xattr storage to `wanix-fs`.

## Consequences

Linux/v86/editor probes now show up as named, typed compatibility requests in
tests and traces, and the server gives deliberate errno responses instead of an
opaque default.

Future cycles can add backing contracts when a client workflow proves that
special files, hard links, or xattrs are required. Until then, Rust Wanix keeps
the public 9P boundary clearer without inventing filesystem semantics too early.
