# ADR 0066: 9P Mutation Operations

## Status

Accepted.

## Context

The Rust 9P path can now browse, stat, read, write, and serve a namespace over
stdio or a native TCP listener. External clients also need the common writable
filesystem operations used by the existing Go p9kit/v86 path:

- `Tlcreate` / `Rlcreate` for creating and opening regular files;
- `Tmkdir` / `Rmkdir` for creating directories;
- `Trenameat` / `Rrenameat` for moving paths between parent directory fids;
- `Tunlinkat` / `Runlinkat` for file removal and directory removal through the
  `AT_REMOVEDIR` flag.

The current Wanix Rust filesystem contract already supports opening with
create/truncate flags, creating directories, removing files/directories, and
renaming paths. It does not yet expose ownership or mode mutation hooks.

## Decision

Add typed 9P2000.L codecs for `Tlcreate`, `Tmkdir`, `Trenameat`, and
`Tunlinkat` in `wanix-protocol`, and map those operations in `wanix-9p` onto the
existing `FileSystem` trait:

- `Tlcreate` joins the supplied basename under the directory fid, opens it with
  `create = true`, and mutates that fid into the opened file on success;
- `Tmkdir` calls `FileSystem::create_dir` and returns the new directory QID;
- `Trenameat` joins source and destination names under their parent directory
  fids and calls `FileSystem::rename`;
- `Tunlinkat` calls `remove_dir` when `AT_REMOVEDIR` is present and
  `remove_file` otherwise.

Mode and gid fields are decoded and preserved in typed request structs, but the
server does not enforce or persist them yet because the backing Rust filesystem
trait has no mode/ownership mutation API.

## Consequences

The native 9P server can now expose a writable host-root demo over stdio or TCP.
This is a direct step toward serve/v86/VS Code integration because clients can
create files, write data, rename paths, and remove entries through the same Rust
server core.

Future work should decide whether Wanix filesystem metadata grows explicit
mode/uid/gid mutation support, and should add browser-facing transport/auth
policy before exposing writable 9P to less trusted clients.
