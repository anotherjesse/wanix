# ADR 0082: 9P Legacy Rename and Remove

## Status

Accepted.

## Context

Rust Wanix already supports the Linux-oriented 9P2000.L mutation messages
`Trenameat` and `Tunlinkat`. Some 9P clients and compatibility probes can still
use the older fid-oriented `Trename` and `Tremove` messages, especially around
mounted filesystem workflows that are relevant to v86, shell, editor, and
future VS Code integration.

Leaving these messages as anonymous unsupported frames makes public transport
traces harder to interpret and prevents the server from applying the expected
fid lifecycle rules.

## Decision

Add dependency-free typed codecs for legacy `Trename`/`Rrename` and
`Tremove`/`Rremove` in `wanix-protocol`.

Handle these messages in `wanix-9p`:

- `Trename` requires both the source fid and destination directory fid to
  exist, validates the destination basename through Wanix path normalization,
  delegates mutation to `FileSystem::rename`, and returns `Rrename` on success.
- Successful renames rebase tracked fids at the source path or below it to the
  new path. If the rename replaces an existing destination, tracked fids at the
  replaced destination path or below it are invalidated.
- `Tremove` requires the fid to exist, removes that fid from the fid table even
  when the backing filesystem removal fails, and returns `Rremove` on success.
- `Tremove` chooses `remove_dir` for directory metadata and `remove_file` for
  file or symlink metadata. This keeps legacy remove closer to Plan 9's
  fid-based remove behavior; Linux-oriented directory removal remains available
  through `Tunlinkat` with `AT_REMOVEDIR`.

The existing `Trenameat` path also rebases tracked source fids and invalidates
replaced destination fids after successful renames.

## Consequences

Rust Wanix now responds deliberately to another class of 9P client mutation
requests over the in-process server, `p9-stdio`, and the `serve`
`/.well-known/export9p` WebSocket route.

Open-file object identity is still simplified because Rust Wanix fid state is
path-backed today. Future cycles can add stronger deleted-but-open object
semantics if a real mounted workflow requires them.
