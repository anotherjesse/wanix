---
title: AttachPolicy — the Trust Boundary as One Pure Function
slug: concepts/attach-policy
pageType: concept
oneLiner: "evaluate(peer, aname) -> Option<Authorization> is the entire attach gate; None denies, and the boundary lives in one auditable place out of the wire code."
audience: [developer, visionary]
tags: [mesh, trust-boundary, shipped, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-id/src/policy.rs:10-45
  - crates/wanix-id/src/grant.rs:8-162
  - crates/wanix-9p/src/lib.rs:125-151
  - crates/wanix-9p/src/session.rs:31-69
  - docs/mesh-the-missing-half-of-9p.md:640-693
seeAlso:
  - concepts/capability-is-a-bind
  - concepts/subtreefs-confine-to-prefix
  - concepts/key-is-the-address
  - concepts/tauth-is-enosys
prerequisites:
  - concepts/capability-is-a-bind
usedInFlows: [{flow: wire-a-mesh, step: 4}]
honestLimits:
  - "Grant lifecycle is partial: the GrantTable supports revoke(), but the #grant service device that would edit it from inside a namespace is designed, not wired."
  - "Revocation takes effect on the next attach; a peer already attached on a live connection keeps its scoped root until it reconnects."
  - "Exec devices reached through a granted root are local-trust only; AttachPolicy scopes which subtree a peer sees, not whether running code inside it is safe."
canonicalCaveatFor: [attach-policy-grant-lifecycle]
---

# AttachPolicy — the Trust Boundary as One Pure Function

`evaluate(peer, aname) -> Option<Authorization>` is the entire attach gate; `None` denies, and the boundary lives in one auditable place out of the wire code.

## What and why

When a peer dials a Wanix node and sends a 9P `Tattach{ aname }`, exactly one question decides what it gets: *who is this, and what may they see?* Wanix answers that with a single trait method — a pure function from a verified peer identity and a requested attach name to an optional authorization. Return a value and the connection installs the scoped root; return `None` and the attach is denied. Because the function is pure and lives in `wanix-id` rather than in the 9P codec, the trust boundary is one small thing you can read, test, and audit, instead of logic smeared across the wire path. This is the structural payoff of [a capability being a bind](/concepts/capability-is-a-bind): the bind only happens if the gate says yes.

## Show: one method, one decision

The whole gate is a one-method trait (`crates/wanix-id/src/policy.rs:10-15`):

```rust
pub trait AttachPolicy: Send + Sync {
    /// Returns the scoped Authorization to install, or None to deny.
    fn evaluate(&self, peer: PeerId, aname: &str) -> Option<Authorization>;
}
```

`peer` is a `PeerId` — a verified identity, not a string the client picked (see [the key is the address](/concepts/key-is-the-address)). `aname` is the attach name the client asked for. The return is `Option<Authorization>`, and `Authorization` is just a scoped root plus its rights (`crates/wanix-id/src/grant.rs:15-21`):

```rust
pub struct Authorization {
    pub root: Arc<dyn FileSystem>,   // the filesystem the peer attaches as its root
    pub rights: Rights,              // the rights enforced on it
}
```

That is the whole contract. No `evaluate` returning a deferred check, no callback into the connection, no peeking at the wire. A policy that knows the peer and the name can decide, and the decision is a value.

## GrantTablePolicy: default-deny, matched exactly

The shipped implementation is `GrantTablePolicy`, which delegates `evaluate` straight to a `GrantTable` (`crates/wanix-id/src/policy.rs:41-45`):

```rust
impl AttachPolicy for GrantTablePolicy {
    fn evaluate(&self, peer: PeerId, aname: &str) -> Option<Authorization> {
        self.grants.authorize(peer, aname)
    }
}
```

A `GrantTable` is a list of `Grant`s, and a `Grant` is "this peer may attach this name and receive this backing filesystem re-rooted at this prefix with these rights" (`crates/wanix-id/src/grant.rs:37-44`). The match is *exact and conjunctive*: a grant applies only when both the peer identity and the attach name are equal (`crates/wanix-id/src/grant.rs:84-88`):

```rust
pub fn matches(&self, peer: PeerId, aname: &str) -> bool {
    self.peer == peer && self.aname == aname
}
```

There are no wildcards. An empty table denies everyone; a populated table denies every `(peer, aname)` pair it does not name. `authorize` walks the grants, takes the first match, and materializes it into an `Authorization` whose root is a [`SubtreeFs`](/concepts/subtreefs-confine-to-prefix) confined to the grant's prefix (`crates/wanix-id/src/grant.rs:95-99`, `:155-161`). If nothing matches, you get `None` — default-deny, with no implicit-allow seam to forget to close. The unit tests pin this down: an empty table denies a `.` attach, and a grant for `(peer, "projects/foo")` denies the same peer attaching `"docs"` and denies a *different* peer attaching `"projects/foo"` (`crates/wanix-id/src/policy.rs:63-96`).

## Name the term: a live-editable grant table

The backing store is `Arc<RwLock<Vec<Grant>>>` (`crates/wanix-id/src/grant.rs:108-111`), and `GrantTable` is `Clone` such that every clone shares one underlying list. That is deliberate: you can hand a clone to the policy that reads it on each attach and keep another clone to mutate. `grant()` pushes, `revoke()` removes every entry matching `(peer, aname)` and returns the count removed, and the policy sees the change on the *next* `evaluate` because it reads the lock fresh each time (`crates/wanix-id/src/grant.rs:120-161`). This is the hook a future `#grant` service file would use to edit authority from inside a namespace — the table is already designed to be edited live; only the device that drives it is unbuilt.

## How the server installs it

The 9P server carries an optional attach context. `P9Server::with_policy(default_root, peer, policy)` builds a server that, on each `Tattach`, calls the policy and either installs the scoped root or denies (`crates/wanix-9p/src/lib.rs:137-151`; `crates/wanix-9p/src/session.rs:58-68`):

```rust
fn install_attach_root(&mut self, aname: &str) -> bool {
    let Some(context) = self.attach.as_ref() else {
        return true;            // no policy: keep the default root, unchanged
    };
    match context.policy.evaluate(context.peer, aname) {
        Some(authorization) => { self.root = authorization.root; true }
        None => false,          // deny
    }
}
```

When `install_attach_root` returns `false`, `handle_attach` replies `Rlerror(EACCES)` and binds no fid (`crates/wanix-9p/src/session.rs:31-35`). Two properties fall out. First, the `peer` comes from the connection — over QUIC, the verified handshake identity (see [9P over iroh QUIC](/concepts/9p-over-iroh-quic)) — never from the client-chosen `uname` in the `Tattach` frame, which a naive server would trust. Second, `with_policy` is strictly additive: a server built *without* a policy returns `true` immediately and keeps its default root, so the unguarded path is byte-for-byte the old server. There is a test asserting the CLI's `build_serve_policy` returns `None` without a `--peer`, so the no-policy path cannot silently acquire one. (Note that 9P `Tauth` itself stays [ENOSYS](/concepts/tauth-is-enosys) — `AttachPolicy` is the gate, not a 9P auth handshake.)

## Why one pure function is auditable

The reason this matters is reviewability. The question "who can see what on this node?" has exactly one answer surface: the `AttachPolicy` and the grants behind it. You do not chase rights checks through fid handling, walk logic, or read paths. The policy is sync, transport-free, and unit-testable over an in-memory backing with no network at all (`crates/wanix-id/src/policy.rs:47-97`). When iroh injects a real `PeerId` from the QUIC connection, not a line of `wanix-id` moves — the boundary was already a function of the peer, and the network just supplies the peer. A trust boundary you can hold in one screen is a trust boundary you can actually trust.

## See also

- [A capability is a bind](/concepts/capability-is-a-bind) — the grant authorizes the bind; this page is the gate that decides.
- [SubtreeFs: confine to a prefix](/concepts/subtreefs-confine-to-prefix) — what the authorized root actually is, and how it stops a symlink escape.
- [The key is the address](/concepts/key-is-the-address) — why `peer` is a verified identity, not a client-claimed name.
- [Tauth is ENOSYS](/concepts/tauth-is-enosys) — `AttachPolicy` gates attach; the 9P auth frame stays unimplemented.
- [Wire a mesh](/learn/wire-a-mesh) — the flow that grants a peer a subtree and watches the gate enforce it.

## Status / honest limits

- **Grant lifecycle is partial.** `GrantTable` already supports `grant()` and `revoke()` over a shared, live-editable store (`crates/wanix-id/src/grant.rs:120-135`), but the `#grant` service device that would let an operator edit grants from inside a namespace is designed, not wired. Today grants are seeded at server construction via `--peer`/`--grant`.
- **Revocation denies the *next* attach.** Because the connection installs one scoped root at attach time (the v1 single-attach simplification, `crates/wanix-9p/src/lib.rs:133-135`), revoking a grant denies the next reconnect; a peer already attached on a live connection keeps its root until it dials again.
- **The gate scopes visibility, not safety.** `AttachPolicy` decides *which subtree* a verified peer attaches. It does not make running code inside that subtree safe: exec devices (`#task`, `#agent`, `#cpu`) reached through a granted root remain local-trust only, with no hard CPU or memory limits. Scoping a peer to a read-only prefix is a real boundary; exposing an exec plane to an untrusted peer is not a supported posture.
