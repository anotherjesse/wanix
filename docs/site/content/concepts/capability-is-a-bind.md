---
title: A Capability Is a Bind
slug: concepts/capability-is-a-bind
pageType: concept
oneLiner: A grant is not an ACL; it re-roots the peer's namespace at a subpath via SubtreeFs gated by read/write rights, so there is no "outside" for them to name.
audience: [visionary, developer]
tags: [mesh, trust-boundary, shipped, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-vfs/src/subtree.rs:58-176
  - crates/wanix-vfs/src/subtree.rs:178-267
  - crates/wanix-id/src/grant.rs:37-161
  - crates/wanix-fs/src/traits.rs:210-231
  - crates/wanix-fs/src/localfs.rs:94-96
  - crates/wanix-9p/src/lib.rs:125-151
  - docs/mesh-the-missing-half-of-9p.md:511-518
  - docs/mesh-the-missing-half-of-9p.md:562-573
seeAlso:
  - concepts/attach-policy
  - concepts/subtreefs-confine-to-prefix
  - concepts/key-is-the-address
  - concepts/trust-boundary-gaps
  - concepts/send-agent-to-the-data
prerequisites:
  - concepts/attach-policy
  - concepts/subtreefs-confine-to-prefix
usedInFlows: []
honestLimits:
  - Exec devices (#task, #agent, #cpu) stay local-trust only; a grant scopes which files a peer sees, not what compute they may run.
  - A connection has one root; v1 installs the most recent authorized attach as the connection root, so per-fid multi-attach scoping is deferred.
  - confine_to_prefix only canonicalizes on host-backed filesystems (LocalFs); MemFs stores link targets opaquely and is a no-op.
canonicalCaveatFor: []
---

# A Capability Is a Bind

A grant is not an ACL; it re-roots the peer's namespace at a subpath via `SubtreeFs` gated by read/write rights, so there is no "outside" for them to name.

In most systems, authorizing a peer means writing down a rule that sits *beside* the data: a list that says "user K may read `projects/foo` but not `secrets/`." Every request then walks the full tree and the rule decides yes-or-no per path. Wanix does the opposite. It hands the peer a different tree. A grant materializes into a filesystem whose root *is* the subtree you gave away, so the peer cannot even name a path you did not grant — there is no parent to walk to. The capability and the namespace are the same object, and that object is the same `FileSystem` everything else in Wanix already trades in.

## Show it: a peer lands inside the subtree, not beside it

A grant says: peer K, attaching the name `projects/foo`, gets the host tree re-rooted at `projects/foo`, read-write. The object that embodies that grant is a `Grant`, and calling `authorize()` on it turns the record into a live filesystem (`crates/wanix-id/src/grant.rs:90-99`):

```rust
pub fn authorize(&self) -> Option<Authorization> {
    let root = SubtreeFs::new(Arc::clone(&self.backing), &self.prefix, self.rights).ok()?;
    Some(Authorization::new(Arc::new(root), self.rights))
}
```

`Authorization { root, rights }` is exactly what the 9P server installs as the attaching fid's root (`crates/wanix-id/src/grant.rs:16-21`). From that moment, the peer is standing *inside* `projects/foo`. They `ls .` and see the contents of `projects/foo`. They open `bar.txt` and they are opening `projects/foo/bar.txt` on your disk. A walk toward `../../docs` does not get filtered or denied by a rule — it simply has nowhere to go, because in the peer's namespace `.` is the subtree and the subtree has no parent. As the mesh design states it: "they cannot name a path outside it because, in their namespace, there is no outside" (`docs/mesh-the-missing-half-of-9p.md:511-518`).

That is the headline, and it is a single sentence: **a capability is a bind.** It is the same `bind`-with-a-source-subpath that the local VFS already does to compose namespaces, plus a permission gate. You do not invent an access-control layer; you give the peer a view.

## Name it: SubtreeFs is the concrete grant

`SubtreeFs` (`crates/wanix-vfs/src/subtree.rs:58-84`) holds three things: an `Arc<dyn FileSystem>` backing, a `NormalizedPath` prefix, and a `Rights`:

```rust
pub struct Rights {
    pub read: bool,
    pub write: bool,
}
```

Every method on `SubtreeFs` does two things before it touches the backing filesystem. First it rebases the incoming path under the prefix — `rebase` simply joins `prefix/path`, and because both are validated `NormalizedPath`s (no `..`, no absolute, no escaping components), the joined path is always inside the prefix (`crates/wanix-vfs/src/subtree.rs:141-149`). Second, it funnels through one central `require` check (`crates/wanix-vfs/src/subtree.rs:123-133`):

```rust
fn require(&self, access: Access) -> FsResult<()> {
    let granted = match access {
        Access::Read => self.rights.read,
        Access::Write => self.rights.write,
    };
    if granted { Ok(()) } else { Err(FsError::PermissionDenied) }
}
```

Reads require `read`; every mutation and every write-mode open requires `write` (`crates/wanix-vfs/src/subtree.rs:168-175`). The bits are additive — granting write does not imply read (`crates/wanix-vfs/src/subtree.rs:22-49`). Because the check lives inside the filesystem and not at the door, the rights are enforced on *every* operation for the whole life of the connection, not validated once at attach and then trusted. A read-only grant that someone tries to write to returns `PermissionDenied` on the `open`, not later, not maybe.

## The symlink escape, and why re-rooting alone is not enough

Re-rooting rewrites the path *string*. That is sufficient on a filesystem that stores symlink targets opaquely and never resolves them as backing paths — `MemFs`, for instance, where there is no shared store to escape into. But on a symlink-*following* backing like `LocalFs` (a real host directory), re-rooting alone has a hole: a symlink that lives inside the granted prefix but whose target points at a sibling *still inside the backing root* would resolve right out of the grant. The string `projects/foo/escape` looks confined; the link `escape -> ../../secrets` is not.

So every symlink-dereferencing method on `SubtreeFs` rebases through `rebase_confined`, which calls the backing's `confine_to_prefix` hook (`crates/wanix-vfs/src/subtree.rs:151-166`):

```rust
fn rebase_confined(&self, path: &NormalizedPath) -> FsResult<NormalizedPath> {
    let rebased = self.rebase(path)?;
    self.backing.confine_to_prefix(&self.prefix, &rebased)?;
    Ok(rebased)
}
```

`confine_to_prefix` defaults to a no-op `Ok(())` for opaque-symlink backings (`crates/wanix-fs/src/traits.rs:229-231`), and `LocalFs` overrides it to canonicalize the host path and reject any target that resolves outside the prefix (`crates/wanix-fs/src/localfs.rs:94-96`). `open`, `metadata` (when following), `read_dir`, and the source side of `hard_link` all route through this confinement; the opaque `read_link` deliberately returns the raw target bytes and is *not* followed (`crates/wanix-vfs/src/subtree.rs:178-228`). The result is that a walk cannot escape the granted subtree even through an in-prefix symlink. The re-rooting gives you string-level confinement for free; `confine_to_prefix` closes the symlink gap that re-rooting alone leaves open.

## Default-deny, keyed by a verified key

A grant is only as good as the identity it names. `GrantTable` is a default-deny list keyed by `PeerId` and attach name (`crates/wanix-id/src/grant.rs:102-161`). There is no wildcard; a `Grant::matches` is an exact `peer == peer && aname == aname` (`crates/wanix-id/src/grant.rs:84-88`). A peer with no matching grant is denied — `authorize` returns `None` and the attach fails with EACCES (`crates/wanix-id/src/grant.rs:155-161`). Nothing is implicitly allowed.

The `PeerId` in that key is not a self-asserted username. On the QUIC transport, the peer's ed25519 key is authenticated *in the connection handshake*, so by the time `authorize(peer, aname)` runs, the identity is already proven — the key *is* the address (see [the key is the address](/concepts/key-is-the-address)). That collapse is why there is no in-band `Tauth`: authentication became a property of the connection, so 9P `Tauth` stays ENOSYS (see [Tauth is ENOSYS](/concepts/tauth-is-enosys)). What remains for the file server is the original Plan 9 job — authorization — and the mechanism it reaches for is the one the system already has for everything: the namespace. The grant table is itself meant to be file-shaped, a structure an agent reads by `ls`, extends by adding a grant, and revokes by removing one (`docs/mesh-the-missing-half-of-9p.md:562-573`).

Because `GrantTable` clones share one underlying list (`crates/wanix-id/src/grant.rs:108-126`), a `#grant`-style device could mutate the live table while the policy reads it — the table is built to be edited at runtime, not frozen at startup.

## See also

- [Attach policy](/concepts/attach-policy) — the `evaluate(peer, aname) -> Authorization` function that is the whole trust boundary.
- [SubtreeFs / confine to prefix](/concepts/subtreefs-confine-to-prefix) — the re-rooting filesystem and the symlink-confinement hook in detail.
- [The key is the address](/concepts/key-is-the-address) — why a verified ed25519 key is both identity and location.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — what the capability model does *not* yet cover.
- [Send the agent to the data](/concepts/send-agent-to-the-data) — the `#cpu` exec plane that runs against a granted namespace.

## Status / honest limits

The capability-is-a-bind boundary is shipped and enforced over real transports (TCP and QUIC), but it is a *file-scoping* boundary, not a compute-isolation boundary. Read it for exactly what it does:

- **A grant scopes files, not compute.** The exec devices `#task`, `#agent`, and `#cpu` are local-trust only and are not exposed to untrusted or public peers. A `SubtreeFs` controls which files a peer can see and mutate; it does not impose CPU or memory limits, and there are none yet. "Cheap, scalable isolation" is the right frame; "safe for arbitrary untrusted code" is not.
- **One root per connection (v1).** `P9Server::with_policy` installs the most recent authorized attach as the connection's root for the rest of the connection (`crates/wanix-9p/src/lib.rs:125-151`). Per-fid root scoping for multiple concurrent attaches on one connection is a deferred fid-namespace change.
- **Symlink confinement depends on the backing.** `confine_to_prefix` only canonicalizes and rejects out-of-prefix targets on host-backed filesystems (`LocalFs`). On opaque-symlink backings (`MemFs`) it is a no-op because there is no shared store to escape into. A new backing that follows symlinks against a shared store must implement the hook or it inherits the escape.
- **A grant is a bind, not an ACL row.** The mental model is "hand out a different tree," not "add a rule to a shared tree." If you find yourself wanting a per-path allow/deny rule, the Wanix answer is usually a narrower prefix or a second grant.
