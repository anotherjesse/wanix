# ADR 0061: 9P Directory Read Cookies

## Status

Accepted.

## Context

The Rust `wanix-9p` server needs directory listing before a native transport
loop is useful for serve, VS Code, v86, or other clients that browse a Wanix
namespace. The Go `fs/p9kit` server implements `Readdir` with monotonically
increasing per-entry cursor cookies, and returns those cookies in each 9P
directory entry's `offset` field.

## Decision

`wanix-protocol` owns dependency-free `Treaddir` and `Rreaddir` codecs for the
9P2000.L wire shape:

- `Treaddir`: `fid[4] offset[8] count[4]`
- `Rreaddir`: `count[4] dirent[count]`
- each `dirent`: `qid[13] offset[8] type[1] name[2+n]`

`wanix-9p` maps `Treaddir` to `FileSystem::read_dir` and uses one-based
per-entry ordinal cookies. A client can resume by passing a previously returned
entry offset. The response never includes a partial directory entry; if the
first entry cannot fit in the requested byte budget, the server returns an empty
entry stream.

Directory `Tlopen` succeeds for read-only flags and returns an `Rlopen` without
creating a byte-file handle, so ordinary 9P clients can open a directory before
issuing `Treaddir`.

## Consequences

This preserves the current Go p9kit cursor behavior while keeping the cookie
opaque enough to change later if a backing filesystem needs stronger state.
Transport loops and broader filesystem operations remain separate follow-up
work.
