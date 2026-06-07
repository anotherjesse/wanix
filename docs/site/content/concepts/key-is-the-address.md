---
title: The Key Is the Address
slug: concepts/key-is-the-address
pageType: concept
oneLiner: A node's persisted ed25519 public key is simultaneously its identity and its dialable iroh EndpointId, so "mount this node" and "trust this node" are the same 32 bytes.
audience: [newcomer, developer, visionary]
tags: [mesh, identity, shipped, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-id/src/identity.rs:30-112
  - crates/wanix-id/src/peer.rs:1-83
  - docs/mesh-the-missing-half-of-9p.md:913-944
  - README.md:42-46
seeAlso:
  - concepts/9p-over-iroh-quic
  - concepts/persisted-ed25519-identity
  - concepts/tauth-is-enosys
  - concepts/capability-is-a-bind
  - concepts/import-export-and-n
  - concepts/one-identity-two-planes
  - recipes/02-mount-remote-peer
prerequisites:
  - concepts/9p-over-iroh-quic
usedInFlows:
  - {flow: wire-a-mesh, step: 2}
honestLimits:
  - In-band 9P Tauth stays ENOSYS forever; identity is proven by the QUIC handshake, not by an auth file exchange.
  - The shipped CLI mount binds a single slot /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is designed-but-unshipped, so /n/<peer> is a labelled convention.
  - Proving identity is not the same as granting access; attach is still default-deny and gated by a capability bind.
---

# The Key Is the Address

A node's persisted ed25519 public key is simultaneously its identity and its dialable iroh EndpointId, so "mount this node" and "trust this node" are the same 32 bytes.

Plan 9's `/n/<node>` convention assumed a network you could name: a known address, a campus LAN you trusted, and a `factotum` daemon holding your keys. None of that survives the modern internet — NATs, no stable addresses, no shared trust. Wanix keeps the Plan 9 *mechanism* and collapses naming, locating, and authenticating into one fact: the node's public key. The same bytes that say *who* a node is also tell iroh *where* to dial it, and the QUIC handshake proves it on the way in.

## The same 32 bytes authorize and locate

Start by looking at what a node *is*. Run the mesh server once and it prints a ticket:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

wanix-rust mesh-serve --root /tmp/world
# -> iroh://<64-hex-char peer-id>?addr=...
```

That `<peer-id>` is the whole identity. It is not a username, not a hostname, not a record in some directory — it is the node's raw ed25519 public key. The `?addr=` suffix is a hint: known direct addresses for a fast first hop (`docs/mesh-the-missing-half-of-9p.md:913-944`). Drop the hint and the key alone is still enough to find the node, because iroh resolves a public key to a live path through relays and DNS discovery.

Now name what just happened. In Plan 9 you would need a *name* (to reach the server) and *credentials* (to authenticate to it). Here the name and the credential are the same 32 bytes. "Mount this node" and "trust this node" are not two steps; they are one address.

## ~/.wanix/node.key: one file, stable across restarts

The key has to come from somewhere durable, or every restart would be a different node. `NodeIdentity::load_or_create` reads it from a file, generating and persisting a fresh one only the first time (`crates/wanix-id/src/identity.rs:72-83`):

```rust
pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self, NodeIdentityError> {
    match std::fs::read(path) {
        Ok(bytes) => Self::from_persisted(&bytes),       // same file -> same key
        Err(err) if err.kind() == NotFound => {
            let identity = Self::generate()?;            // first run: mint + persist
            identity.persist(path)?;
            Ok(identity)
        }
        Err(err) => Err(NodeIdentityError::Io(err.to_string())),
    }
}
```

The default path is `~/.wanix/node.key` (`identity.rs:90-93`), and on Unix it is written `0600` — owner read/write only (`identity.rs:157-170`). What lands on disk is the raw 32-byte ed25519 *seed*, serialized exactly as `to_secret_bytes` produces it (`identity.rs:44-48`), deliberately *not* wrapped in an iroh re-export the blueprint warns is a private pre-release format. The round trip is its own inverse: write `to_secret_bytes`, read `from_secret_bytes`, get the same key. The `Debug` impl prints only the public `peer_id` and never the secret (`identity.rs:114-121`).

The consequence is the property the mesh needs: the same key file always yields the same public key, so a node keeps its address — and its trust relationships — across restarts.

## PeerId: equality by key, hex for humans

The public half is a `PeerId` — a thin newtype over `[u8; 32]` (`crates/wanix-id/src/peer.rs:10-11`). Three properties matter:

- **It is the raw public key.** `from_bytes`/`as_bytes` carry exactly the 32 ed25519 public-key bytes — no envelope, no salt (`peer.rs:13-24`).
- **Equality is by key.** `PeerId` derives `Eq`/`Ord`/`Hash`, so two peers are the same peer iff their key bytes match (`peer.rs:10`, test at `peer.rs:75-82`). The grant table is keyed by this, *never* by a client-claimed `uname` (`peer.rs:5-9`).
- **Hex is the display form.** `to_hex` is the 64-character lowercase rendering you copy-paste; `Display` and `Debug` both use it (`peer.rs:26-55`). The hex is for your eyes; the bytes are the truth.

So the `<peer-id>` in the ticket above is just `PeerId::to_hex()`. When you mount a peer, you hand back those bytes, and iroh dials the node they name.

## Why this makes /n/<node> work on the open internet

Two Plan 9 daemons collapse into the iroh handshake (`docs/mesh-the-missing-half-of-9p.md:922-938`):

**factotum becomes a property of the connection.** Plan 9 split authentication into a separate daemon so no file server had to speak auth protocols. iroh goes one better: the QUIC handshake authenticates the peer's ed25519 key *in the transport*. By the time a connection is accepted, the remote identity is already proven, and the server reads it straight off the connection as a verified `PeerId` (`peer.rs:5-9`) — it never trusts a name the client typed.

**The node id is simultaneously the identity and the address.** This is what makes `/n/<node>` workable across NATs you could never have addressed directly. The same key that authorizes a node also lets iroh *find* it; a ticket is that key plus optional direct-address hints (`README.md:42-46`). An agent that mounts a node in another building, authenticated by its key and confined by a grant, is the mesh doing its job — by `cat`, `write`, and `ls`, over QUIC.

That is the foundation for [send the agent to the data](/concepts/send-agent-to-the-data): a node you can reach, an identity that gates what it exposes, and a stream you can reverse-export a namespace over.

## See also

- [9P over iroh QUIC](/concepts/9p-over-iroh-quic) — the transport that carries the proven identity and the 9P frames.
- [Persisted ed25519 identity](/concepts/persisted-ed25519-identity) — the key-file lifecycle in depth.
- [Tauth is ENOSYS](/concepts/tauth-is-enosys) — why there is no in-band 9P auth handshake.
- [A capability is a bind](/concepts/capability-is-a-bind) — proving identity is not the same as granting reach.
- [Attach policy](/concepts/attach-policy) — default-deny grants keyed by `PeerId`.
- [Import, export, and /n](/concepts/import-export-and-n) — the Plan 9 mount convention this realizes.
- [Recipe 02: mount a remote peer](/recipes/02-mount-remote-peer) — dial a node and run a job on it.

## Status / honest limits

- **There is no in-band 9P auth.** `Tauth` stays `ENOSYS` forever (`docs/mesh-the-missing-half-of-9p.md:929`). Identity is established once, in the QUIC handshake; the 9P layer above it never re-proves anything.
- **`/n/<peer>` is a convention, not a per-peer mount table yet.** The shipped CLI `mount` verbs bind a *single* slot, `/n/remote` (`crates/wanix-cli/src/mount.rs:26`); per-peer `/n/<peer-id>` slots are designed-but-unshipped. Write `/n/<peer>` only as a labelled convention, not as a path that resolves today.
- **Proven is not the same as permitted.** A verified `PeerId` says only *who* dialed. Whether that peer may attach — and to what subtree — is a separate, default-deny decision made by a capability bind. See [attach policy](/concepts/attach-policy).
