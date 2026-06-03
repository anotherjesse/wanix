# ADR 0077: 9P Open Append Semantics

## Status

Accepted

## Context

Rust Wanix is widening 9P compatibility for Linux/v86, browser, and editor
clients. Those clients can open a regular file with the Linux `O_APPEND` flag
through 9P2000.L `Tlopen` or `Tlcreate`, then issue `Twrite` requests whose
offsets are not meaningful for append-mode files.

The Go p9kit oracle stores append mode on the opened file and writes at the
current end of file when `O_APPEND` is present.

## Decision

Track append mode as opened fid state in `wanix-9p`:

- `Tlopen` records whether the fid was opened with `O_APPEND`.
- `Tlcreate` records append mode on the created-and-opened fid.
- `Twrite` seeks to end of file before writing when the fid is append-opened,
  ignoring the request offset.
- Read-only directory opens continue to work even if the client includes
  `O_APPEND`; directory fids do not store append state because they are not
  writable file handles.

This does not add an append flag to the core `wanix-fs::OpenOptions` contract.
The append behavior is protocol open state owned by the 9P adapter.

## Consequences

External 9P clients can append to files through Rust Wanix exports in the same
shape as the Go implementation, which improves mounted workflow compatibility
for shells, editors, logs, and package tools.

Future filesystem-specific atomic append guarantees remain out of scope. The
current server preserves append semantics for a single in-process 9P server and
its opened fid state.
