# ADR 0020: Expose Read-Only Virtual WASI Filesystem

## Status

Accepted

## Context

QuickJS-NG's standard library can observe WASI filesystem imports when a host
provides them. Until now, this prototype exposed deterministic clock/random,
stdio, callbacks, modules, and other host policy, but filesystem access remained
unsupported. Hosts need a small way to provide configuration files, bundled
source, fixtures, or other immutable guest-visible bytes without mounting host
paths or serializing Rust-side file handles into snapshots.

Filesystem access is a trust boundary: guest paths are attacker-controlled byte
strings, WASI file descriptors are host state, and open descriptors cannot be
faithfully represented by the existing linear-memory snapshot format.

## Decision

Add read-only virtual files to `QuickJsHostConfig`. Hosts register immutable
byte contents at absolute normalized guest paths with
`with_read_only_virtual_file` or `with_read_only_virtual_files`.

The runtime exposes a narrow WASI Preview 1 surface when at least one virtual
file is configured:

- fd `3` is a read-only root preopen named `/`.
- `path_open` only accepts relative normalized paths under that root.
- Guest paths with absolute prefixes, empty components, `.`, `..`, NUL bytes, or
  backslashes are rejected before lookup.
- Absolute configured paths are capped at 4096 bytes, and `path_open` rejects
  relative paths that would exceed that cap after adding the root `/`.
- `path_open` rejects mutation-like flags and rejects rights outside read, seek,
  tell, and filestat.
- `fd_filestat_get` and `path_filestat_get` report copied metadata for virtual
  files: file type and byte size. The root preopen reports as a directory and
  stdio reports as character devices. Parent paths implied by configured files
  also report as virtual directories, but remain non-enumerable.
- File contents are copied into host-owned immutable storage at configuration
  time and copied into guest memory on `fd_read`.
- `fd_seek`, `fd_close`, and `fd_fdstat_get` operate only on virtual file
  descriptors plus the already-known stdio/preopen descriptors.

Snapshots do not serialize the virtual filesystem config or open file
descriptors. Create and restore attach the configured virtual files exactly like
other `QuickJsHostConfig` policy. Snapshotting fails while any virtual file
descriptor is open so resume cannot lose descriptor position or rights state.

## Consequences

- Hosts can provide guest-visible read-only files without exposing host paths,
  directories, symlinks, or ambient filesystem authority.
- `QuickJsHostConfig` debug output reports only the number of configured virtual
  files, not their paths or contents.
- Restore remains explicit: callers supply the virtual file map again when they
  need post-restore reads, and snapshot bytes remain a QuickJS VM image rather
  than a bundle of host resources.
- The first filesystem capability is intentionally not a general WASI
  implementation. Directory enumeration, mutable files, host path mounts,
  symlinks, mutable metadata, and custom preopen layout remain future decisions.
- Long-running guests must close virtual file descriptors before snapshotting.
  A future snapshot format could encode descriptor state, but this ADR keeps the
  current format smaller and avoids pretending that Rust host handles live in
  WebAssembly memory.
