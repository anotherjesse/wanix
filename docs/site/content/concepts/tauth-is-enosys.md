---
title: Tauth Is ENOSYS (Identity Is a Transport Property)
slug: concepts/tauth-is-enosys
pageType: concept
oneLiner: In-band 9P authentication is deliberately unimplemented; the QUIC handshake authenticates the peer's key in the transport, so factotum becomes a property of the connection.
audience: [developer, visionary]
tags: [mesh, trust-boundary, identity, shipped, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-9p/src/session.rs:71-74
  - crates/wanix-mesh/src/handler.rs:101-135
  - crates/wanix-mesh/src/identity.rs:23-31
  - AGENTS.md:246-247
seeAlso:
  - concepts/key-is-the-address
  - concepts/attach-policy
  - concepts/the-9p-contract
  - concepts/trust-boundary-gaps
prerequisites:
  - concepts/key-is-the-address
usedInFlows: []
honestLimits:
  - Over plain stdio or TCP (p9-stdio, p9-listen, the local serve websocket) there is no transport identity at all; those paths are local-trust only.
  - handle_attach decodes uname/aname but discards uname entirely; the trust gate keys on the QUIC-verified PeerId, not on anything the client claims in-band.
  - The Tauth path returns ENOSYS unconditionally — there is no 9P auth-file handshake to fall back to.
canonicalCaveatFor: [tauth-enosys]
---

# Tauth Is ENOSYS (Identity Is a Transport Property)

In-band 9P authentication is deliberately unimplemented; the QUIC handshake authenticates the peer's key in the transport, so factotum becomes a property of the connection.

Send a `Tauth` request to a Wanix 9P server and you get back `ENOSYS` — "function not implemented." That is not a gap waiting to be filled. It is the design. Wanix never authenticates *inside* the 9P conversation, because by the time a `Tattach` arrives over the mesh, the peer's ed25519 key has already been proven by the QUIC handshake one layer down. Identity lives below 9P, not inside it. This page explains why that is the right place for it, and exactly where in the code the decision is made.

## Show it: Tauth gets ENOSYS

Here is the entire `Tauth` handler (`crates/wanix-9p/src/session.rs:71-74`):

```rust
pub(super) fn handle_auth(&mut self, frame: &P9Frame) -> Result<P9Frame, Wanix9pError> {
    p9_decode_tauth(frame)?;
    Ok(p9_rlerror(frame.tag(), ENOSYS))
}
```

It decodes the frame just far enough to reply on the right tag, then refuses. There is no auth fid, no challenge, no shared-secret exchange. A 9P client that expects to negotiate credentials through an auth-file gets a clean, honest "no such mechanism here." The client is meant to attach directly — and whether that attach is *allowed* is decided by who is on the other end of the wire, which 9P never asked about.

## Why factotum maps to the QUIC handshake

Plan 9 solved authentication with **factotum**, an agent that held your keys and spoke an auth protocol *over* the 9P connection through a special auth file negotiated by `Tauth`/`Rauth`. The mechanism was in-band: 9P carried both the data and the credentials to reach it.

Wanix moves that responsibility down a layer. The mesh transport is 9P over iroh QUIC, and QUIC's TLS handshake already proves the remote peer holds the private key for a specific ed25519 public key. That public key *is* the peer's identity — the same key that addresses the node ([key is the address](/concepts/key-is-the-address)). So the cryptographic work factotum used to do in-band is now done by the connection itself, before a single 9P byte flows. There is nothing left for `Tauth` to negotiate. Authentication became a property of the transport, and 9P went back to being purely a file protocol.

## The gate keys on the verified PeerId, never on uname

The proof that identity comes from below 9P is in where the trust decision reads its input. On the mesh side, the protocol handler reads the peer key straight from the verified QUIC handshake before any 9P frame is served (`crates/wanix-mesh/src/handler.rs:101-135`):

```rust
// Identity is read from the verified QUIC handshake. 0-RTT is not used
// (we never call `into_0rtt`), so `remote_id` is the proven peer key
// before any Tattach is served.
let peer = peer_id_for(connection.remote_id());
```

That `peer` — the ed25519 public key, lifted out of the QUIC TLS certificate (`crates/wanix-mesh/src/identity.rs:23-31`) — is what the server carries into every attach. Contrast it with the 9P attach handler (`crates/wanix-9p/src/session.rs`): `handle_attach` decodes the `Tattach` frame, which *does* carry a client-supplied `uname` (the username) and `aname` (the attach name). Wanix uses `aname` to select a scoped subtree, but `uname` is decoded and thrown away. The doc comment on the identity helper says it plainly: the grant table is keyed by "the cryptographic identity… never a client-claimed `uname`."

This is the whole point of pushing identity below 9P. A client can *claim* to be anyone in its `uname` field; that claim is free and unverifiable. The QUIC key cannot be claimed — it is proven by possession of a private key. So the [attach policy](/concepts/attach-policy) evaluates the verified `peer` against `aname` and either installs a scoped root or replies `EACCES`. The username never enters the trust calculation. A capability here is a *bind* — an authorized peer attaches a re-rooted subtree — not an ACL check on a name.

## When there is no transport identity at all

The flip side of "identity is a transport property" is blunt: a transport with no identity confers no identity. When `ServeConfig::open` exports a root with `policy: None`, the namespace is exported wholesale, "exactly as an unguarded `p9-listen`" (`crates/wanix-mesh/src/handler.rs:39-48`). Over plain stdio (`p9-stdio`), raw TCP (`p9-listen`), or the local serve websocket, there is no QUIC handshake, so there is no proven key, so there is nothing for a policy to gate on. Those paths are **local-trust only**. Tauth being ENOSYS does not make them safer — it just means the missing authentication is honestly absent rather than faked by a half-built in-band scheme. Real peer authentication exists only on the QUIC mesh edge, and only there does the attach policy have a verified principal to evaluate.

## See also

- [Key is the address](/concepts/key-is-the-address) — why the ed25519 public key is both the name and the credential of a node.
- [Attach policy](/concepts/attach-policy) — how the verified PeerId plus `aname` resolve to a scoped subtree or `EACCES`.
- [The 9P contract](/concepts/the-9p-contract) — the frame, fid, and message set that `Tauth`/`Tattach` belong to.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — what mesh authentication does and does not yet cover.

## Status / honest limits

- **Tauth is ENOSYS unconditionally.** There is no fallback in-band auth handshake; a client expecting an auth file will not find one (`crates/wanix-9p/src/session.rs:71-74`).
- **`uname` is decoded and discarded.** The trust gate keys exclusively on the QUIC-verified `PeerId`, never on the client-claimed username (`crates/wanix-mesh/src/identity.rs:23-31`).
- **No transport identity off the QUIC edge.** `p9-stdio`, `p9-listen`, and the local serve websocket have no handshake-proven peer key, so they are local-trust only and any attach policy there has nothing to evaluate (`crates/wanix-mesh/src/handler.rs:39-48`, `AGENTS.md:246-247`).
- **Public/multi-user auth is explicitly unimplemented.** Beyond the per-attach key gate, broader multi-user and public-facing authentication remains open trust-boundary work (`AGENTS.md:246-247`).
