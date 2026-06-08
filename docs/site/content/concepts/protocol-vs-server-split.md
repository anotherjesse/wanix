---
title: Protocol vs Server Split
slug: concepts/protocol-vs-server-split
pageType: concept
oneLiner: wanix-protocol is dependency-free wire codecs; wanix-9p maps fids to FileSystem objects and Linux errnos — transports are thin adapters over the same server.
audience: [developer]
tags: [9p, protocol, server, crate-layering, shipped, caveat]
sourceRefs:
  - crates/wanix-protocol/src/p9.rs:1-39
  - crates/wanix-protocol/src/p9/frame.rs:239-257
  - crates/wanix-9p/src/lib.rs:1-53
  - crates/wanix-9p/src/error.rs:51-66
  - crates/wanix-9p/src/session.rs:21-28
  - crates/wanix-9p/src/session.rs:71-73
  - crates/wanix-9p/src/transport.rs:1-40
  - docs/adrs/0004-rust-9p-protocol-and-server-contract.md:19-48
seeAlso:
  - concepts/the-9p-contract
  - concepts/remotefs-import-half
  - reference/crate-map-and-layering
prerequisites:
  - concepts/the-9p-contract
usedInFlows: []
honestLimits:
  - "Rust Wanix does not fake auth, special-file, xattr, ownership, inode-link, or device semantics beyond what the FileSystem contract can provide; Tauth returns ENOSYS."
  - "serve handles one 9P frame at a time per connection, so a blocking read cannot interleave with a write on the same connection."
canonicalCaveatFor: []
---

# Protocol vs Server Split

`wanix-protocol` is dependency-free wire codecs; `wanix-9p` maps fids to `FileSystem` objects and Linux errnos — transports are thin adapters over the same server.

9P is one protocol doing two jobs: turning bytes on a wire into typed messages, and answering those messages against a real filesystem. Wanix keeps those jobs in two crates that never bleed into each other. The bottom crate knows nothing about Wanix; the middle crate knows nothing about sockets. That seam is why adding a new transport — TCP, WebSocket, a QUIC stream — is a few dozen lines of plumbing, not a new 9P implementation.

## Two crates, one cut

Open the two crate headers and the split is stated outright. `wanix-protocol` is "intentionally below Wanix filesystem policy: callers can split byte streams into tagged 9P frames and decode typed payloads" (`crates/wanix-protocol/src/p9.rs:1-5`). `wanix-9p` "maps typed `wanix-protocol` 9P frames onto `wanix-fs` filesystems. It owns fid state and filesystem error mapping, while the protocol crate remains dependency-free and wire-only" (`crates/wanix-9p/src/lib.rs:1-5`).

So: bytes are the protocol crate's problem; meaning is the server crate's problem. ADR 0004 makes this the durable contract — `wanix-protocol` owns "frame splitting, tag extraction, version negotiation, and typed operation codecs," and `wanix-9p` owns "the server state that maps fids to Wanix filesystem objects and open-file state" (`docs/adrs/0004-rust-9p-protocol-and-server-contract.md:19-29`).

## wanix-protocol: bytes in, typed frames out

The protocol crate is pure wire mechanics. It splits a byte stream into framed 9P messages, reads the tag, negotiates the version, and decodes each operation into a typed payload — and it depends on nothing else in the workspace.

Frame splitting is the visible primitive. A `P9FrameBuffer` accumulates bytes and hands back every complete frame so far, leaving a partial frame buffered for the next read (`crates/wanix-protocol/src/p9/frame.rs:239-257`):

```rust
pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<P9Frame>, P9Error> {
    self.bytes.extend_from_slice(bytes);
    let mut frames = Vec::new();
    while let Some(size) = p9_declared_size(&self.bytes)? {
        if self.bytes.len() < size {
            break;          // partial frame — wait for more bytes
        }
        let frame = P9Frame::decode(&self.bytes[..size])?;
        self.bytes.drain(..size);
        frames.push(frame);
    }
    Ok(frames)
}
```

`p9_declared_size` reads the leading u32 size field; `p9_tag_from_frame_bytes` pulls the tag without a full decode (`crates/wanix-protocol/src/p9/frame.rs`). On top of that sit typed op codecs — `p9_tversion`/`p9_rversion` and friends build and decode each message (`crates/wanix-protocol/src/p9.rs:51-58`), and the crate carries the server-facing 9P2000.L surface plus selected Google compatibility codecs (`9P2000.L.Google.1`/`.2`) for `walkgetattr`-aware clients. Errors are wire-shaped, not filesystem-shaped: `ShortFrame`, `InvalidFrameSize`, `FrameSizeMismatch`, `UnexpectedEof`, `InvalidUtf8` (`crates/wanix-protocol/src/p9/frame.rs:7-68`). Nothing here knows what a `#kv` key is.

## wanix-9p: fids to FileSystem, errors to errnos

The server crate is where a frame becomes an action against a real `FileSystem`. It owns the per-connection state the protocol crate deliberately lacks: a `BTreeMap` of fids to filesystem positions and open-file handles, the negotiated msize, and the negotiated Google version (`crates/wanix-9p/src/lib.rs:18-53`). A `Twalk` moves a fid to a new path; a `Topen` opens a `Box<dyn File>` and parks it on that fid; a `Tread` reads from the parked handle.

The other half of the server's job is translating `FsError` into the Linux-ish errno a 9P2000.L client expects (`crates/wanix-9p/src/error.rs:51-66`):

```rust
pub(crate) fn errno_for_fs(error: &FsError) -> u32 {
    match error {
        FsError::NotFound => 2,                 // ENOENT
        FsError::NotSupported => EOPNOTSUPP,     // 95
        FsError::PermissionDenied => EACCES,     // 13
        FsError::AlreadyExists => EEXIST,        // 17
        FsError::IsDirectory => EISDIR,          // 21
        // ...
    }
}
```

This is the seam in one function. A device that leaves `write` at its `NotSupported` default (see [the FileSystem trait](/concepts/the-filesystem-trait)) becomes an `Rlerror` carrying `EOPNOTSUPP` on the wire — without the device, the protocol crate, or the transport knowing anything about each other. Version negotiation lives here too: the server clamps the client's msize to its own ceiling and picks the highest compatible dialect, falling back to plain `9P2000.L` (`crates/wanix-9p/src/session.rs:21-28`).

## Transports preserve frame boundaries; diagnostics stay out

A transport's only job is to move framed bytes and to keep its own logging out of the binary stream. The synchronous stream server reads in 8 KiB chunks, feeds them to a `P9FrameBuffer`, dispatches each completed frame through the server, and writes the encoded reply back (`crates/wanix-9p/src/transport.rs:1-40`). `P9TransportStats` counts requests, responses, and bytes; `P9TransportError` separates an I/O failure from a `Protocol` decode failure from a `Server` rejection — so a malformed frame is diagnosable without poisoning the channel.

ADR 0004 makes the boundary rule explicit: stdio, TCP, WebSocket, and `serve` are "adapters over the same server contract. They must preserve binary frame boundaries and keep diagnostic/status output out of binary response streams" (`docs/adrs/0004-rust-9p-protocol-and-server-contract.md:41-43`). There is one per-connection session core — `P9Server::serve_duplex<D: Read + Write>`, with `serve_stream` delegating to it — and every transport is a thin byte adapter over it: `wanix-rust p9-stdio` (the process pipe), the websocket door and the raw-TCP `--p9` door on `serve`, and the iroh QUIC stream the mesh uses. They differ only in their byte plumbing, never in what `Tread` means. (The standalone `p9-listen`/`p9-ws` subcommands were retired into `serve` under ADR 0006; the websocket door is a framing adapter, not a second server.)

## A new transport is a thin adapter, not a fork

Pin the consequence. To carry 9P over a new medium you supply two things: a reader that delivers bytes into a `P9FrameBuffer`, and a writer that flushes encoded reply frames. The frame split, tag extraction, version negotiation, fid table, and errno mapping are already done and shared. That is exactly how the mesh got network transparency: `wanix-mesh` carries the *same* synchronous 9P core over an iroh QUIC stream, and `wanix-9p-client`'s `RemoteFs` (see [the import half](/concepts/remotefs-import-half)) reuses the same protocol crate to *speak* 9P as a client. One protocol stack, many wires — because the wire and the meaning were never tangled. The layering rule that keeps it that way is in the [crate map](/reference/crate-map-and-layering).

## See also

- [The 9P contract](/concepts/the-9p-contract) — what 9P is and the operations Wanix serves.
- [RemoteFs: the import half](/concepts/remotefs-import-half) — the client side reusing the same protocol crate.
- [Crate map and layering](/reference/crate-map-and-layering) — the dependency direction that keeps the seam clean.
- [The FileSystem trait](/concepts/the-filesystem-trait) — where `NotSupported` becomes `EOPNOTSUPP`.

## Status / honest limits

- **Wanix does not fake semantics it cannot back.** Unsupported features return deliberate protocol errors rather than plausible-looking lies. `Tauth` returns `ENOSYS` — there is no 9P auth handshake (`crates/wanix-9p/src/session.rs:71-73`); auth lives at the mesh transport layer, not in the 9P session. Per ADR 0004, the server "should not fake auth, special-file, extended attribute, ownership, inode-link, or device semantics beyond what the Wanix filesystem contract can actually provide" (`docs/adrs/0004-rust-9p-protocol-and-server-contract.md:45-48`).
- **One frame at a time per connection.** The synchronous transport dispatches each decoded frame before reading the next, so on a single `serve` connection a blocking read (such as a `#plumb/<topic>/recv`) cannot interleave with a write on that same connection — live pub/sub needs a second connection or concurrent frame handling. The frame split is honest; the concurrency is still maturing.
- **Compatibility codecs are demand-driven.** The protocol crate carries the server-facing 9P2000.L surface and only the Google extensions that real clients (browser cockpit, v86) actually request; it is not a complete implementation of every 9P dialect.
