---
title: Persisted ed25519 Node Identity
slug: concepts/persisted-ed25519-identity
pageType: concept
oneLiner: A 32-byte ed25519 seed at ~/.wanix/node.key (0600, stable across restarts) is the node identity; the QUIC handshake authenticates it so in-band Tauth stays ENOSYS.
audience: [developer]
tags: [mesh, identity, shipped, trust-boundary, local-trust-only]
sourceRefs:
  - crates/wanix-id/src/identity.rs:72-111
  - crates/wanix-id/src/identity.rs:36-48
  - crates/wanix-mesh/src/handler.rs:101-135
  - crates/wanix-mesh/src/identity.rs:19-31
  - docs/recipes/02-mount-remote-peer.md:25-31
seeAlso:
  - concepts/key-is-the-address
  - concepts/tauth-is-enosys
  - concepts/attach-policy
  - concepts/trust-boundary-gaps
prerequisites:
  - concepts/key-is-the-address
usedInFlows: []
honestLimits:
  - No public, multi-user authentication exists; the mesh authenticates peer keys, not human accounts.
  - Per-principal namespaces are designed but unfinished — every fid still resolves through one shared served root.
  - The shipped CLI mount binds a single slot /n/remote; per-peer /n/<peer-id> is a labelled convention, not shipped wiring.
canonicalCaveatFor: []
---

# Persisted ed25519 Node Identity

A 32-byte ed25519 seed at `~/.wanix/node.key` (mode `0600`, stable across restarts) is the node identity; the QUIC handshake authenticates it, so in-band Tauth stays ENOSYS.

A Wanix node does not log in. It *is* a keypair. The first time it needs an identity it generates a 32-byte ed25519 seed, writes it owner-private to disk, and from then on the same file always yields the same public key — and that public key is its address on the mesh. There is no account server, no username, no password file. The mesh's transport (QUIC) proves who is on the other end of the wire, which means the 9P layer never has to run its own authentication handshake. This page is about where that one file lives, why it is exactly 32 raw bytes, and why the server reads the *connection's* proven identity instead of anything a client claims.

## Show: one file, generated once

There is nothing to register. The node loads its key, or makes one:

```rust
// crates/wanix-id/src/identity.rs:72-83
let identity = NodeIdentity::load_or_create(NodeIdentity::default_key_path()?)?;
```

`default_key_path()` returns `~/.wanix/node.key` (`crates/wanix-id/src/identity.rs:90-93`). `load_or_create` reads the file; on `NotFound` it generates a fresh identity and persists it. From the recipe's two-node setup you simply point each node at its own key path and a stable identity falls out (`docs/recipes/02-mount-remote-peer.md:25-31`):

```sh
export NODE_A_KEY=/tmp/wanix-A.key
export NODE_B_KEY=/tmp/wanix-B.key
# First run creates each file (0600); every later run reuses it.
```

The public key derived from that seed is the node's `PeerId` (`crates/wanix-id/src/identity.rs:58-60`). Restart the process and the address is unchanged — peers that learned it once can still find and verify you. That stability is the whole point of persisting rather than regenerating: a regenerated key is a new node.

## Name: the seed is the identity, stored raw

The structure carries exactly one secret — an ed25519 `SigningKey` — and exposes it as raw bytes (`crates/wanix-id/src/identity.rs:36-48`):

```rust
pub fn from_secret_bytes(seed: [u8; 32]) -> Self { /* SigningKey::from_bytes */ }
pub fn to_secret_bytes(&self) -> [u8; 32] { self.signing.to_bytes() }
```

The round-trip is deliberately *just* those 32 bytes. The crate doc spells out why: the seed is serialized raw, not through an iroh re-export, because relying on iroh re-exporting `ed25519-dalek` would couple persistence to a private pre-release type (`crates/wanix-id/src/identity.rs:12-21`). So the on-disk format is the ed25519 primitive itself, and the same 32 bytes hand verbatim to iroh's `SecretKey` when the mesh binds its endpoint (`crates/wanix-mesh/src/identity.rs:19-21`). The iroh endpoint's identity is therefore byte-for-byte the persisted Wanix identity, and its public key *is* the `PeerId` — there is no second identity to keep in sync. (See [key is the address](/concepts/key-is-the-address) and [one identity, two planes](/concepts/one-identity-two-planes).)

A persisted key file that is not exactly 32 bytes is rejected with `BadLength` rather than silently truncated (`crates/wanix-id/src/identity.rs:95-100`) — a corrupted or wrong-format file fails loudly instead of producing a different node.

## Why 0600, and why never the secret in Debug

The seed is the node's whole authority: anyone who can read it can *be* the node. So the writer opens the file with mode `0600` on Unix — owner read/write, nobody else (`crates/wanix-id/src/identity.rs:157-170`). The parent `~/.wanix` directory is created on demand. `Debug` for `NodeIdentity` prints only the `peer_id`, never the secret (`crates/wanix-id/src/identity.rs:114-121`), so a stray log line cannot leak the key. These are small disciplines, but they are the trust boundary for a key that has no revocation story yet.

## The server trusts the connection, not the client's claim

Here is where persisted identity meets the wire. When the mesh accepts a 9P-over-QUIC connection, it reads the cryptographically verified peer key straight from the handshake — never a client-supplied `uname` (`crates/wanix-mesh/src/handler.rs:101-106`):

```rust
// 0-RTT is not used (we never call into_0rtt), so remote_id is the proven
// peer key before any Tattach is served.
let peer = peer_id_for(connection.remote_id());
```

That `peer` is what every attach is gated against by the `AttachPolicy` (`crates/wanix-mesh/src/handler.rs:145-148`; see [attach policy](/concepts/attach-policy)). Because QUIC's TLS handshake already proved the peer's ed25519 public key, 9P has nothing left to authenticate in-band: a `Tauth` message gets ENOSYS and no auth fid is ever negotiated. The persisted key plus the transport handshake *is* the authentication. (See [Tauth is ENOSYS](/concepts/tauth-is-enosys).) Refusing 0-RTT matters here: 0-RTT data can be replayed, so the handler deliberately never calls `into_0rtt`, and `remote_id` is the proven key before a single byte of 9P is served.

## See also

- [Key is the address](/concepts/key-is-the-address) — why the public key, not a DNS name or IP, names a node.
- [Tauth is ENOSYS](/concepts/tauth-is-enosys) — what the transport handshake replaces and why 9P auth stays unimplemented.
- [Attach policy](/concepts/attach-policy) — how the verified peer key gates a default-deny attach.
- [One identity, two planes](/concepts/one-identity-two-planes) — the same key drives both the 9P import plane and the `#cpu` exec plane.
- [Trust-boundary gaps](/concepts/trust-boundary-gaps) — what authentication does *not* yet cover.
- [Mount a remote peer](/recipes/02-mount-remote-peer) — the two-node walkthrough that uses these key files.

## Status / honest limits

- **Peer keys, not human accounts.** The mesh authenticates a node's ed25519 key. There is no public, multi-user authentication — no login, no account directory, no per-user credentials. Authenticating *who pressed the keys* is unbuilt trust-boundary work.
- **Per-principal namespaces are unfinished.** The verified `peer` is read on every connection, but the served root is still shared: `handle_attach` decodes `uname`/`aname` and every fid resolves through one root. Scoping a peer to its own re-rooted view is designed (`AttachPolicy` + `SubtreeFs`) but the per-fid root storage is not finished.
- **`/n/<peer>` is a convention, not shipped routing.** The shipped CLI mount binds a single slot `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Writing a peer-keyed path like `/n/<peer-id>/#kv/...` is a labelled convention in these docs; the multi-slot, per-peer mount table is designed but not yet wired.
- **No revocation or rotation.** A leaked `node.key` cannot be revoked; rotating it produces a new node identity that peers must re-learn.
