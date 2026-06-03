# ADR 0073: 9P Symlink and Readlink

## Context

Rust Wanix already owns symbolic-link behavior in `wanix-fs`, `wanix-vfs`, and
live Wanix-backed WASI. The Rust 9P server did not expose those semantics yet,
which left browser/v86 and future editor clients unable to observe or create
symlinks through the native filesystem export.

Linux 9P clients use 9P2000.L `Tsymlink` and `Treadlink` for these operations.
Fid identity also needs no-follow metadata so a walked symlink is reported as a
link, not as the file it points at.

## Decision

Add 9P2000.L `Tsymlink`/`Rsymlink` and `Treadlink`/`Rreadlink` codecs in
`wanix-protocol`, and handle them in `wanix-9p`:

- `Tsymlink` resolves the supplied directory fid, creates the new link with
  `FileSystem::symlink`, and replies with the symlink QID.
- `Treadlink` reads the uninterpreted target bytes with `FileSystem::read_link`
  and encodes them as the 9P target string.
- invalid or non-link readlink requests map through the existing filesystem
  errno mapping.
- fid QID and `Tgetattr` metadata use no-follow metadata so symlinks stay
  visible as links.

Opening a symlink still follows the existing backing filesystem open behavior.

## Consequences

Rust `serve` and other 9P transports can now expose host-backed symlinks to
external clients, and clients can create links through the same Wanix-owned
filesystem boundary used by WASI. This is another step toward real v86/Linux
and editor workflows over the Rust 9P server.

The current wire contract encodes link targets as 9P strings. If a future
backing filesystem needs arbitrary non-UTF-8 targets over 9P, the server should
decide an explicit error or encoding policy instead of silently changing this
contract.
