---
title: The 9P Contract
slug: concepts/the-9p-contract
pageType: concept
oneLiner: 9P2000.L (plus the Google.1/.2 extensions) is the foreign-edge wire that lets Linux, v86, editors, and browsers browse and mutate any Wanix namespace; between two Wanix nodes the mesh now uses the native FileSystem-over-iroh wire instead.
audience: [developer, visionary]
tags: [shipped, foreign-edge, mesh, protocol, caveat, trust-boundary]
sourceRefs:
  - docs/adrs/0004-rust-9p-protocol-and-server-contract.md
  - crates/wanix-9p/src/dispatch.rs:1-101
  - crates/wanix-9p/src/lib.rs:84-93
  - crates/wanix-9p/src/session.rs:21-74
  - crates/wanix-9p/src/session.rs:144-161
  - crates/wanix-9p/src/error.rs:51-60
  - crates/wanix-protocol/src/p9/message.rs:1-8
seeAlso:
  - concepts/protocol-vs-server-split
  - concepts/remotefs-import-half
  - concepts/tauth-is-enosys
  - concepts/missing-half-of-9p
  - concepts/trust-boundary-gaps
prerequisites:
  - concepts/everything-is-a-file
usedInFlows: []
honestLimits:
  - There is no in-band 9P authentication; Tauth always returns ENOSYS and identity is carried by the transport, not the protocol.
  - The plain server decodes Tattach's uname/aname and discards them in the no-policy path; every fid resolves through one shared root until a per-principal namespace seam lands.
  - The server speaks a server-facing 9P2000.L subset plus selected Google extensions; it does not fake auth, device nodes, xattrs, or POSIX ownership beyond what the backing filesystem provides.
---

# The 9P Contract

9P2000.L (plus the Google.1/.2 extensions) is the foreign-edge wire that lets Linux, v86, editors, and browsers browse and mutate any Wanix namespace.

[Everything is a file](/concepts/everything-is-a-file) is the *shape* of Wanix; 9P is one *wire* that carries that shape off the machine — the one a *foreign* peer already speaks. Once every capability is a `FileSystem`, a single protocol reaches all of them, and it is the same protocol whether the client is a Linux guest, a browser cockpit, or an editor. This page pins down the operation set Wanix actually serves, how version negotiation works, why authentication is deliberately absent from the wire, and what the server refuses to fake.

> **9P is the foreign edge, not the mesh wire.** Between two *Wanix* nodes — both of which speak the `FileSystem` trait natively — the mesh now uses the [native FileSystem-over-iroh wire](/concepts/missing-half-of-9p), not 9P, because tunnelling 9P between two trait-speakers buys nothing and costs chattiness, head-of-line blocking, tags, and `msize`. 9P stays exactly where a peer genuinely needs it: Linux `v9fs`, v86/QEMU virtio-9p, external 9P tools, and the browser cockpit's `p9.ts`. [ADR 0004](/reference/adr-index) records this split — one `FileSystem` contract, the native wire inward, 9P at the foreign edge.

## One protocol, every external client

Start the server over standard streams:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

wanix p9-stdio --root .
```

That speaks 9P frames on stdin/stdout. The same server reaches foreign clients over `wanix serve` — which carries 9P both over a WebSocket door (`/.well-known/export9p`) and, with `--p9 HOST:PORT`, over a raw TCP door — and over iroh QUIC on the foreign-edge ALPN (`wanix/9p/1`) for a peer that speaks only 9P. The transports differ; the contract does not. There is exactly one per-connection 9P session core (`P9Server::serve_duplex`); every transport is a thin byte adapter over it, not a second server (the standalone `p9-listen`/`p9-ws` subcommands were retired and folded into `serve` under ADR 0006). ADR 0004 states it plainly: stdio, TCP, WebSocket, and `serve` are *adapters over the same server contract*, and each must preserve binary frame boundaries and keep diagnostics out of the binary stream (`docs/adrs/0004-rust-9p-protocol-and-server-contract.md`). A Linux `mount -t 9p`, a v86 guest, and the VS Code workbench all hit `P9Server::handle_frame`; none of them invent filesystem semantics of their own. (A *Wanix* mesh peer does not — it uses the native wire's `serve_one` dispatch instead, against the same `FileSystem`.)

That one entry point is the whole server. `handle_frame` dispatches a decoded request through five groups — session, walk, I/O, metadata, mutation — and any message type that matches none of them returns `EOPNOTSUPP` (`crates/wanix-9p/src/dispatch.rs:23-40`). The grouping is just code organization; to a client it is one flat operation set.

## The supported operation set

A 9P session is a fixed dance: negotiate a version, attach to a root, walk to a path, open or create a handle, then read/write/readdir until you clunk it. The dispatcher names every operation Wanix answers (`crates/wanix-9p/src/dispatch.rs:42-100`):

- **Session:** `Tversion`, `Tauth`, `Tattach`, `Tflush`, `Tflushf`, `Tfsync`, `Tclunk`.
- **Walk:** `Twalk` and `Twalkgetattr` (the Google.2 fast-path that fetches attributes in the same round trip).
- **I/O:** `Tlopen`, `Tlcreate`, `Tread`, `Twrite`, `Treaddir`.
- **Metadata:** `Tstatfs`, `Treadlink`, `Tgetattr`, `Tsetattr`, `Txattrwalk`, `Txattrcreate`, `Tlock`, `Tgetlock`.
- **Mutation:** `Tsymlink`, `Tmknod`, `Tlink`, `Tmkdir`, `Trename`, `Trenameat`, `Tremove`, `Tunlinkat`.

This is the *Linux* dialect of 9P — 9P2000.L, not the original 9P2000 — which is why the verbs are `Tlopen`/`Tlcreate` and why `Tgetattr`/`Tsetattr` carry Linux stat fields. The split between the dependency-free wire codec and the filesystem-backed server is its own design point; see [protocol vs server split](/concepts/protocol-vs-server-split).

Where the backing filesystem can do something, the server does it for real: `Tmkdir`, `Trenameat`, and `Tunlinkat` mutate a `MemFs` or a host directory, and on a host-backed root `Tsymlink`/`Treadlink`/`Tlink` create and read real links. Where it cannot, the server returns a deliberate error rather than a lie — `Tmknod` and `Tlink` against an in-memory root answer `EOPNOTSUPP`, and a `Txattrwalk` for `user.foo` does too, because Wanix has no extended-attribute backing to offer. ADR 0004 makes this a rule: *unsupported features return deliberate protocol errors until Wanix has a backing contract*, and the server "should not fake auth, special-file, extended attribute, ownership, inode-link, or device semantics."

## Version negotiation rejects what it does not speak

`Tversion` is the first frame, and it both resets the session and pins the dialect (`crates/wanix-9p/src/session.rs:21-29`). The server clears all fids, clamps the client's requested `msize` down to its own ceiling, and runs the requested version string through `negotiate_p9_version` (`crates/wanix-9p/src/session.rs:144-161`):

```rust
fn negotiate_p9_version(requested: &str) -> (&'static str, u32) {
    if requested == P9_VERSION_9P2000_L {
        return (P9_VERSION_9P2000_L, 0);
    }
    // "9P2000.L.Google.<n>" -> highest extension level <= n
    // anything else -> ("unknown", 0)
}
```

A bare `9P2000.L` request gets `9P2000.L`. A `9P2000.L.Google.2` request gets Google.2, which unlocks `Twalkgetattr`; `Google.1` unlocks `Tflushf`. A version string the server does not recognize is answered with the literal `"unknown"` in the `Rversion` reply — the 9P-standard way a server says *no* (`crates/wanix-protocol/src/p9/message.rs:1-8`). The client must then re-offer a version both sides share or give up; it never proceeds on a dialect the server did not agree to. The extension level is also load-bearing later: a `Tflushf` arriving before the session negotiated Google.1 is answered `EOPNOTSUPP`, not silently honored (`crates/wanix-9p/src/session.rs:81-91`).

## Tauth is ENOSYS — identity is a transport property

Open a session and you will find one operation that *never* succeeds. `Tauth` decodes the request and immediately replies `ENOSYS`, binding no fid (`crates/wanix-9p/src/session.rs:71-74`):

```rust
pub(super) fn handle_auth(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
    p9_decode_tauth(frame)?;
    Ok(p9_rlerror(frame.tag(), ENOSYS))
}
```

This is a decision, not a gap. Wanix does not run a 9P authentication handshake; there is no afid, no auth file, no in-band credential exchange. Identity lives one layer down, in the transport. On the mesh, the QUIC connection is mutually authenticated by ed25519 node keys before a single 9P frame is read, so the peer is already known when `Tattach` arrives — the protocol does not need to re-establish it. That is why [Tauth is ENOSYS](/concepts/tauth-is-enosys) is the canonical home of this caveat, and why the capability check happens at attach, not auth: a [capability is a bind](/concepts/capability-is-a-bind) decided by the [attach policy](/concepts/attach-policy), keyed on the transport-verified peer.

## Errors map to Linux errnos; the same server runs everywhere

Every failure crosses the wire as an `Rlerror` carrying a Linux errno. The server translates filesystem results through one function (`crates/wanix-9p/src/error.rs:51-60`): `NotFound` becomes `ENOENT`, `NotSupported` becomes `EOPNOTSUPP`, `PermissionDenied` becomes `EACCES`, `IsDirectory`/`NotDirectory` become `EISDIR`/`ENOTDIR`, an unknown fid becomes `EBADF`. A Linux guest that mounted the namespace sees ordinary `errno` values from ordinary syscalls; nothing about Wanix leaks into the error path. Because the mapping and the operation set are identical across stdio, TCP, WebSocket, and `serve`, a script developed against `p9-stdio` behaves the same when a browser drives it over HTTP or a peer drives it over QUIC.

## See also

- [Protocol vs server split](/concepts/protocol-vs-server-split) — `wanix-protocol` owns the wire codec; `wanix-9p` owns the filesystem-backed server state.
- [RemoteFs: the import half](/concepts/remotefs-import-half) — the client side of this contract, which mounts a remote 9P export as a local `FileSystem`.
- [Tauth is ENOSYS](/concepts/tauth-is-enosys) — why authentication is a transport property, not a 9P frame.
- [The missing half of 9P](/concepts/missing-half-of-9p) — import as the operation that turns one protocol into a mesh.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — what the session/namespace seam still discards, and what that means today.

## Status / honest limits

- **No in-band authentication.** `Tauth` always returns `ENOSYS` (`crates/wanix-9p/src/session.rs:71-74`). A client never authenticates *through* 9P; it must already be the peer the transport verified. Public/multi-user 9P auth remains explicitly unimplemented.
- **The session/namespace seam discards uname/aname in the plain path.** A `P9Server::new` server decodes `Tattach`'s `uname` and `aname` and then resolves every fid through one shared `root` (`crates/wanix-9p/src/lib.rs:84-93`, `crates/wanix-9p/src/session.rs:31-69`). A policy-carrying server scopes the root *per connection* on the first authorized attach, but per-fid root storage for multiple concurrent attaches is a deferred change, so per-principal namespaces are designed-but-unshipped.
- **A subset, served honestly.** The server speaks a server-facing 9P2000.L subset plus the Google.1/.2 extensions, and returns deliberate `EOPNOTSUPP`/`ENOSYS` errors for device nodes, extended attributes, hard links on backings that lack them, and anything else the backing `FileSystem` cannot provide (`docs/adrs/0004-rust-9p-protocol-and-server-contract.md`). It does not fabricate POSIX semantics it cannot honor.
