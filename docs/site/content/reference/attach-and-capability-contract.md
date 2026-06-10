---
title: Attach & capability contract
slug: reference/attach-and-capability-contract
pageType: reference
oneLiner: The flat spec for how a peer attaches and what a capability grant authorizes.
audience: [developer]
tags: [mesh, trust-boundary, shipped, local-trust-only, caveat, cli]
sourceRefs:
  - crates/wanix-id/src/policy.rs:10-45
  - crates/wanix-id/src/grant.rs:16-161
  - crates/wanix-9p/src/lib.rs:73-151
  - crates/wanix-9p/src/session.rs:31-74
  - crates/wanix-cli/src/serve/raw9p/grant.rs:24-115
  - crates/wanix-cli/src/serve/command/options.rs:11-16
  - crates/wanix-cli/src/mesh/serve.rs:30-149
  - crates/wanix-vfs/src/subtree.rs:14-49
  - crates/wanix-mesh/src/dialer.rs:62-101
seeAlso:
  - concepts/capability-is-a-bind
  - concepts/attach-policy
  - concepts/subtreefs-confine-to-prefix
  - concepts/tauth-is-enosys
  - concepts/key-is-the-address
  - devices/kv
  - reference/cli-command-index
prerequisites: []
usedInFlows: []
honestLimits:
  - Per-principal namespaces and grant lifecycle are not yet implemented; the v1 server keeps the most recent authorized attach per connection (crates/wanix-9p/src/lib.rs:133-135).
  - The shipped CLI mount binds a single slot /n/remote (crates/wanix-cli/src/mount.rs:26); per-peer /n/<peer-id> is a labelled convention, not a code path.
  - Tauth always replies ENOSYS; there is no 9P auth handshake (crates/wanix-9p/src/session.rs:71-74).
canonicalCaveatFor: []
---

# Attach & capability contract

The flat spec for how a peer attaches and what a capability grant authorizes.

When a remote node mounts your namespace, exactly one decision gates everything that follows: which subtree does this peer get, and may it write? Wanix answers that with a pure function — `evaluate(peer, aname)` — and a default-deny table of grants. There are no ACLs walked per file, no per-method permission lattice, no `uname` to spoof. A capability **is** a bind: a grant materializes into a re-rooted, rights-gated `SubtreeFs` that the 9P server installs as the attaching fid's root. This page is the contract: the signature, the grant grammar, the keying, and the v1 limits that are real today versus the multi-principal shape that is designed but unshipped.

## The signature

The whole trust boundary funnels through one trait (`crates/wanix-id/src/policy.rs:10-15`):

```rust
pub trait AttachPolicy: Send + Sync {
    fn evaluate(&self, peer: PeerId, aname: &str) -> Option<Authorization>;
}
```

`peer` is a verified ed25519 identity; `aname` is the attach name the client sent in its `Tattach`. Returning `Some(Authorization)` installs a scoped root; returning `None` denies the attach. That is the entire decision surface. Keeping it a pure function of `(peer, aname)` is deliberate: "who gets which root" lives in one auditable place, out of the wire code (`policy.rs:6-9`).

An `Authorization` is two fields — the filesystem to attach and the rights enforced on it (`crates/wanix-id/src/grant.rs:16-21`):

```rust
pub struct Authorization {
    pub root: Arc<dyn FileSystem>,
    pub rights: Rights,
}
```

## Default-deny GrantTable

The shipped policy is `GrantTablePolicy`, backed by a `GrantTable` (`policy.rs:22-45`). A `Grant` is the durable record "this peer may attach this `aname` and receive `backing` re-rooted at `prefix` with `rights`" (`grant.rs:37-44`). Matching is exact — peer identity **and** attach name — with no wildcard, in keeping with default-deny (`grant.rs:84-88`). A peer with no matching grant gets `None`; there is no implicit allow (`grant.rs:108-161`).

The table is cheaply cloneable and shares one backing `Vec` behind an `RwLock`, so the same table can be revoked from while the policy reads it on each attach (`grant.rs:108-135`). `revoke(peer, aname)` removes every matching grant and returns how many it dropped — the mechanism a future `#grant` service file would drive. The lookup itself is first-match-wins (`grant.rs:149-161`), so add more specific grants before broader ones.

## A capability is a SubtreeFs

`Grant::authorize` materializes the capability (`grant.rs:96-99`):

```rust
let root = SubtreeFs::new(Arc::clone(&self.backing), &self.prefix, self.rights).ok()?;
Some(Authorization::new(Arc::new(root), self.rights))
```

A `SubtreeFs` is the backing filesystem re-rooted at `prefix` and gated by `Rights` (`crates/wanix-vfs/src/subtree.rs:58-83`). The peer's `.` maps to `prefix`; every path it presents is rebased under that prefix and run through one central rights check. Crucially the rights are enforced *inside* the filesystem, not only at attach time — so a granted peer cannot walk out of its subtree, and a read-only grant denies every mutation with `PermissionDenied`. On a symlink-following backing, each dereferencing method also calls `confine_to_prefix`, which canonicalizes and rejects any target outside the prefix, so an in-prefix symlink can't escape the grant (`subtree.rs:67-79`). See [a capability is a bind](/concepts/capability-is-a-bind) and [SubtreeFs confines to a prefix](/concepts/subtreefs-confine-to-prefix).

`Rights` is two booleans — `read` and `write` — with `read_only()`, `read_write()`, and `none()` constructors (`subtree.rs:14-49`). There is no `exec` bit here. Exec is not a right you grant on a subtree; it is a *device* you choose to bind into the served namespace, and the served paths refuse to bind the exec devices on any public endpoint (see honest limits).

## Verified-PeerId keying

The grant key is a cryptographic identity, never a client-claimed string. On the QUIC mesh, the handler reads the proven peer key from the verified handshake before any `Tattach` is served, and never uses a client `uname` (`crates/wanix-mesh/src/handler.rs:102-106`). On the serve raw-9P door (`serve --p9`) there is no handshake to verify a key, so the operator supplies the peer explicitly with `--peer HEX` (`crates/wanix-cli/src/serve/raw9p/grant.rs:9-16`). The two paths share the grant grammar and the `PeerId` keying, but they differ in trust: on the mesh the peer is cryptographically proven, while over raw TCP `--peer` is an *operator assertion*, not proof. Either way, the `PeerId` that keys the grant table is the address — see [the key is the address](/concepts/key-is-the-address).

## Tauth = ENOSYS

There is no 9P authentication handshake. `Tauth` always replies `ENOSYS` (`crates/wanix-9p/src/session.rs:71-74`). Identity is established by the transport (the QUIC handshake), not by an in-protocol auth file. See [Tauth is ENOSYS](/concepts/tauth-is-enosys).

## How attach installs the root

`handle_attach` decodes the `Tattach`, calls `install_attach_root(aname)`, and on denial replies `EACCES` and binds no fid (`session.rs:31-69`). A server built with `P9Server::new` carries no policy and installs the default root wholesale — the unguarded, loopback-trusted case (`crates/wanix-9p/src/lib.rs:107-123`). A server built with `P9Server::with_policy` carries the verified peer and the policy; a matching grant replaces the connection root with the scoped one (`lib.rs:136-151`).

The mesh dialer is the matching import half: `dial_attach(addr, aname)` sends a specific attach name to import a scoped capability, while `dial` sends the empty root `aname` for the unscoped case (`crates/wanix-mesh/src/dialer.rs:62-101`). The plain TCP `mount-*` verbs always attach the default root and discard `aname`.

## serve `--p9` grant grammar

Raw 9P over TCP is a serve mode (`serve --p9 ADDR`), not a separate daemon — the retired `p9-listen`/`p9-ws` subcommands folded their grant/policy plumbing into the serve raw-9P door unchanged. The CLI grant spec is a colon triple — `ANAME:PREFIX:RIGHTS` (`crates/wanix-cli/src/serve/raw9p/grant.rs:24-58`):

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

wanix-rust serve --root . --p9 127.0.0.1:9999 \
  --peer <64-hex-ed25519-pubkey> \
  --grant projects/foo:projects/foo:rw \
  --grant docs:docs:ro
```

`RIGHTS` is `rw` or `ro` and nothing else; `ANAME` and `PREFIX` are Wanix paths that may contain `/` but not `:` (`grant.rs:34-52`). `--peer` is exactly 64 hex digits (`grant.rs:71-85`). `--grant` repeats; every spec is scoped to the serve `--p9` root (`grant.rs:104-115`). `--grant` without `--peer` is a usage error, and a policy without `--p9` is a usage error. The `--p9` door defaults to loopback; over raw TCP `--peer` is asserted by the operator, not cryptographically proven. The mesh `mesh-serve` path takes the same `--peer`/`--grant` grammar over QUIC, where the peer key *is* proven by the handshake (`crates/wanix-cli/src/mesh/serve.rs:46-91`).

## --insecure-open

`mesh-serve` on the public endpoint with no `--peer`/`--grant` is refused, because that would export the whole root read-write to anyone holding the ticket — inverting default-deny on a global transport (`mesh/serve.rs:106-119`). `--insecure-open` is the explicit opt-in to do exactly that. It exports the host directory **read-write, but never the exec devices** (`crates/wanix-cli/src/help.rs:43`); `--wanix-services` (which binds `#task`/`#agent`) stays refused on any public endpoint regardless of grants or `--insecure-open` (`mesh/serve.rs:120-139`).

## See also

- [A capability is a bind](/concepts/capability-is-a-bind) and [SubtreeFs confines to a prefix](/concepts/subtreefs-confine-to-prefix) — what an `Authorization` becomes.
- [AttachPolicy](/concepts/attach-policy) — the single-place trust decision.
- [The key is the address](/concepts/key-is-the-address) and [Tauth is ENOSYS](/concepts/tauth-is-enosys) — verified-identity keying with no in-protocol auth.
- [`#kv`](/devices/kv) — a device that imports across the mesh once a grant lets the peer attach.
- [CLI command index](/reference/cli-command-index) — `serve --p9`, the `mount-*` verbs, and `mesh-serve` in full.

## Status / honest limits

- **Single attach per connection (v1).** The most recent authorized `Tattach` defines the connection's root; the server overwrites `self.root` on each successful attach (`crates/wanix-9p/src/lib.rs:133-135`, `session.rs:58-69`). Per-fid root scoping for multiple concurrent attaches — and the per-principal namespaces and grant-lifecycle work that build on it — is a deferred fid-namespace change, designed but unshipped.
- **`/n/<peer>` is a convention, not a code path.** The shipped CLI mount binds one slot, `/n/remote` (`crates/wanix-cli/src/mount.rs:26`). Per-peer `/n/<peer-id>` is designed-but-unshipped; use `/n/<peer>` only as a label.
- **Exec devices are local-trust only.** `--wanix-services` binds `#task`/`#agent` (remote code execution). On the mesh it is refused on any non-loopback endpoint even with grants or `--insecure-open` (`crates/wanix-cli/src/mesh/serve.rs`; only a loopback `--addr 127.0.0.1:PORT` qualifies, since a LAN `--addr` is mDNS-discoverable); on `serve` it is refused whenever either 9P door (the websocket door on the HTTP listener, or the raw `--p9` door) is bound non-loopback (ADR 0006). Grants gate *which subtree*; they do not make exec safe for untrusted peers, and there are no hard CPU/memory limits yet.
- **No 9P auth.** `Tauth` is `ENOSYS` (`session.rs:71-74`); identity comes from the transport handshake, so the raw-TCP serve `--p9` door requires an out-of-band `--peer HEX` — and over raw TCP that `--peer` is asserted, not proven. Only the iroh QUIC mesh edge binds and verifies a NodeID, so cross-machine cryptographic trust stays the mesh's job.
