---
title: Five Hostile-Peer Corrections
slug: concepts/five-hostile-peer-corrections
pageType: concept
oneLiner: Frame-size ceiling vs OOM, honest seekability vs silent corruption, RAII fid/tag guards vs leaks, bounded read_dir, and server-side append — the difference between a 9P importer that demos and one you would trust against a node you do not control.
audience: [developer]
tags: [mesh, the-9p-contract, caveat, shipped]
sourceRefs:
  - crates/wanix-9p-client/src/transport.rs:34-65
  - crates/wanix-9p-client/src/file.rs:131-180
  - crates/wanix-9p-client/src/fid.rs:115-166
  - crates/wanix-9p-client/src/readdir.rs:1-67
  - crates/wanix-9p-client/src/remote.rs:135-219
  - docs/mesh-the-missing-half-of-9p.md:210-297
seeAlso:
  - concepts/remotefs-import-half
  - concepts/the-9p-contract
  - concepts/capability-is-a-bind
prerequisites:
  - concepts/remotefs-import-half
usedInFlows: []
honestLimits:
  - "These corrections harden the importer against a hostile or buggy peer at the application layer; transport-level authentication and confinement live elsewhere (9P-over-iroh QUIC and the grant table)."
  - "Tauth stays ENOSYS: there is no in-band 9P auth handshake. Identity is proven by the QUIC transport, not by the client."
  - "read_dir is a best-effort snapshot, not an atomic view; without the Google.2 walkgetattr batch, per-entry metadata is listing-only (type only, size/mode placeholders)."
---

# Five Hostile-Peer Corrections

Frame-size ceiling vs OOM, honest seekability vs silent corruption, RAII fid/tag guards vs leaks, bounded `read_dir`, and server-side append — the difference between a 9P importer that demos and one you would trust against a node you do not control.

A first draft of a 9P client is easy to write and easy to get subtly, dangerously wrong. The moment you `bind` a `RemoteFs` at `/n/<peer>` and start resolving paths into it, the server on the other end is no longer "your" server — it is whatever node you mounted, and on a mesh that node may be buggy, overloaded, or actively hostile. The threat model flips: the importer is the trusting party, and every reply it reads is attacker-controlled bytes. `wanix-9p-client` carries five corrections, each a fix for a specific way the obvious implementation betrays the importer. None are stylistic. Each was a real adversarial verdict against the naive code.

## Why a hostile server is the threat model

Plan 9's import is "splice a remote tree into my namespace and treat it as local files." That is exactly the property that makes a remote server dangerous: once bound, a `Tread` on `/n/peer/x` is an ordinary file read to every caller above it, but on the wire it is an R-message the peer wrote. A client that assumes the server is honest will allocate what the server tells it to, track offsets the server ignores, and loop as long as the server keeps talking. The five corrections below replace each of those assumptions with a bound or a check, so a misbehaving peer degrades into an error instead of taking the importer down with it.

## 1. Frame-size ceiling: bound the allocation before reading the body

The naive client reads the 4-byte size prefix, does `vec![0; size]`, then fills it. A hostile server declares a 1 GiB frame and the importer allocates a gigabyte before it has read a single body byte — a one-line denial of service. The corrected `read_one_frame` reads the prefix first and rejects any declared size below the 9P header length or above the negotiated `msize` *before* allocating the body (`crates/wanix-9p-client/src/transport.rs:47-65`):

```rust
if size > max_msize {
    return Err(ClientError::Poisoned(format!(
        "server declared a {size}-byte frame exceeding the negotiated msize {max_msize}"
    )));
}
```

`msize` was agreed during `Tversion`; the server cannot retroactively exceed it. An importer must not be DoS-able by the thing it imports.

## 2. Honest seekability: a Tgetattr at open decides regular-file

Regular files are offset-addressable; devices and service streams are not — the server reads them sequentially and ignores `Tread.offset`. A naive client tracks a client-side offset for everything and advances it on every read. Point that at `#term/<id>/ctl` and you fabricate offsets the server discards, then *believe* your own fiction — silent corruption that looks like data. The corrected `open_impl` issues a `Tgetattr` on the freshly opened fid and only marks the file seekable if the server reports it a regular file (`crates/wanix-9p-client/src/remote.rs:135-219`). A non-regular file reports `is_seekable() == false` and *refuses* `seek` with `NotSupported` rather than inventing a position (`crates/wanix-9p-client/src/file.rs:140-167`):

```rust
fn seek(&mut self, from: FileSeekFrom) -> FsResult<u64> {
    if !self.seekable {
        return Err(FsError::NotSupported);
    }
    ...
}
```

The reader half follows the same rule: the tracked offset only advances on a seekable file (`file.rs:89-91`). A service stream stays at a stable zero, which is the truth.

## 3. RAII fid/tag guards: clunk on every drop path, including panic

Every walk allocates a fid on the server. A long-lived mount issues an enormous number of walks, opens, and reads. If any error path between "walk produced a fid" and "the operation consumed it" forgets to clunk — including a panic unwinding through the caller — that fid leaks into the server's map forever, and over a long session you exhaust the fid space. `ScratchFid` is the leak guard: it holds a clone of the shared connection and best-effort clunks its fid on `Drop`, on *every* exit path (`crates/wanix-9p-client/src/fid.rs:115-166`):

```rust
impl Drop for ScratchFid {
    fn drop(&mut self) {
        if self.released { return; }
        if let Ok(mut conn) = self.conn.lock() {
            conn.clunk_fid(self.fid);
        }
    }
}
```

When ownership legitimately transfers to an open `RemoteFile`, `into_fid()` sets `released` so the guard defuses; the fid is then clunked exactly once, by the file handle, when the file drops (`fid.rs:148-152`, `file.rs:174-180`). The fid and tag pools recycle released numbers through free-lists so a long session never wraps `u32`/`u16` or reuses the `NOFID`/`NOTAG` sentinels.

## 4. Bounded read_dir: cap entries, cap iterations, bail on a stalled cookie

A naive `read_dir` either reads one page and lies about completeness, or loops forever if a broken server never advances its cookie. The corrected `read_all_entries` is a bounded `Treaddir` cookie loop: it pages until an empty page, caps total entries at `MAX_ENTRIES` and total round trips at `MAX_ITERATIONS`, and bails if the cookie fails to advance (`crates/wanix-9p-client/src/readdir.rs:39-66`):

```rust
if last_cookie == cookie {
    // A server that fails to advance the cookie would loop forever;
    // stop rather than trust a stalled stream.
    break;
}
```

The listing is documented as a best-effort snapshot, not an atomic view, and the per-entry metadata is honestly listing-only — file type from the directory-entry byte, with size and mode as placeholders — unless a Google.2 `Twalkgetattr` batch supplies more (`readdir.rs:1-13`). No pretending a directory browse is a free per-file stat.

## 5. O_APPEND delegated to the server, never raced client-side

To append, the naive client stats the file for its length, then writes at that offset — and races any other writer between the stat and the write. The corrected client passes `O_APPEND` to `Tlopen` and lets the *server* seek to end before each write (`crates/wanix-9p-client/src/remote.rs:141-143`). The client sends a stable zero offset, which an append fid ignores server-side, rather than computing a position it cannot trust (`crates/wanix-9p-client/src/file.rs:95-110`):

```rust
// Append fids let the server seek to end; the offset field is ignored
// server-side, so the client sends a stable zero rather than racing a
// size probe.
let offset = if self.append { 0 } else { self.offset };
```

Append is the server's job because only the server has a coherent view of the file's end across concurrent writers. The client declines to guess.

## The common shape

Read the five together and one discipline emerges: **trust the negotiated bound, not the per-message claim.** `msize` was agreed once and caps every frame. A `Tgetattr` decides seekability once and the offset logic respects it. A fid is owned by exactly one guard whose `Drop` cannot be skipped. A directory walk has a ceiling. An append authority lives where the truth lives. Each correction moves a decision the server could lie about into a place the importer controls — and that is what separates a client that demos against `127.0.0.1` from one you would mount across a mesh you do not own.

## See also

- [RemoteFs: the import half](/concepts/remotefs-import-half) — the `FileSystem` that these corrections live inside.
- [The 9P contract](/concepts/the-9p-contract) — the message set and the negotiated `msize` these bounds rest on.
- [A capability is a bind](/concepts/capability-is-a-bind) — the transport-layer confinement that complements these application-layer guards.
- [9P over iroh QUIC](/concepts/9p-over-iroh-quic) — where the peer identity is actually proven.

## Status / honest limits

- These corrections harden the importer against a hostile or buggy *peer* at the application layer. They do not authenticate the peer or confine what it may reach — that is the transport's and the grant table's job. See [9P over iroh QUIC](/concepts/9p-over-iroh-quic) and [a capability is a bind](/concepts/capability-is-a-bind).
- `Tauth` stays `ENOSYS`. There is no in-band 9P auth handshake; identity is a property of the QUIC connection, supplied by the transport, never claimed by the client `uname`.
- `read_dir` is a best-effort snapshot, not an atomic view. Concurrent mutation on the server can shift entries between pages, and without the Google.2 `Twalkgetattr` extension per-entry metadata is listing-only (type only; size and mode are placeholders) — sufficient for a browse, not a substitute for a per-file `metadata` call.
- The frame ceiling protects against an oversized declared frame; it does not protect against a server that simply stalls mid-body. A premature EOF surfaces as a transport error, but a hung connection still blocks the serial RPC until the underlying stream times out.
