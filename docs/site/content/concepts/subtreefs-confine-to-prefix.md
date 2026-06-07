---
title: SubtreeFs / confine_to_prefix
slug: concepts/subtreefs-confine-to-prefix
pageType: concept
oneLiner: SubtreeFs re-roots a backing FS at a prefix and gates every method through one central require(right) check; confine_to_prefix closes the symlink-escape hole.
audience: [developer]
tags: [capability, trust-boundary, mesh, shipped]
sourceRefs:
  - crates/wanix-vfs/src/subtree.rs:58-176
  - crates/wanix-vfs/src/subtree.rs:178-267
  - crates/wanix-fs/src/traits.rs:207-231
  - crates/wanix-fs/src/localfs.rs:94-96
  - crates/wanix-fs/src/localfs/paths.rs:27-40
  - crates/wanix-vfs/src/subtree/tests.rs:207-263
seeAlso:
  - concepts/capability-is-a-bind
  - concepts/attach-policy
  - concepts/host-not-ambient-authority
  - concepts/normalizedpath
  - concepts/namespace-binding
prerequisites:
  - concepts/namespace-binding
  - concepts/normalizedpath
usedInFlows:
  - {flow: wire-a-mesh, step: 4}
honestLimits:
  - Confinement is enforced per backing filesystem, not globally; each symlink-following backing must override confine_to_prefix or its grants leak.
  - On an opaque-symlink backing (MemFs) confine_to_prefix is a deliberate no-op, because that backing never resolves a link target as a backing path and so cannot suffer the escape.
  - SubtreeFs gates read vs write; it does not enforce any CPU, memory, or rate limit, and exec devices behind it stay local-trust only.
canonicalCaveatFor: []
---

# SubtreeFs / confine_to_prefix

SubtreeFs re-roots a backing FS at a prefix and gates every method through one central `require(right)` check; `confine_to_prefix` closes the symlink-escape hole.

When a peer is granted `projects/foo` read-write, Wanix does not hand it the whole disk and trust it to stay put. It hands it a *different filesystem* — one whose root is `projects/foo`, whose every method first asks "is this access permitted?", and whose symlink resolution is fenced so a clever link cannot walk out of the grant. That filesystem is `SubtreeFs`, and it is the concrete object a capability grant becomes.

## Show: a grant that cannot reach past its prefix

A grant of `projects/foo` read-write produces a `SubtreeFs` whose `.` maps to `projects/foo` in the backing filesystem. From the peer's side every path is subtree-relative — it opens `report.md`, not `projects/foo/report.md` — and the rebase happens underneath:

```text
peer sees:   report.md            escape          ..
SubtreeFs:   projects/foo/report.md   PermissionDenied   (rejected: not a NormalizedPath)
```

Reading `report.md` works. A `..` never even forms a path: the incoming path is a `NormalizedPath`, so `../../secret` is rejected at construction before any rebase (`crates/wanix-vfs/src/subtree.rs:96-107`). And a symlink named `escape` whose target points up and out of the prefix is denied — the test plants `../../docs/secret.txt` inside the granted prefix and confirms the open returns `PermissionDenied`, not the secret (`crates/wanix-vfs/src/subtree/tests.rs:207-263`).

## The central `require` check

`SubtreeFs` carries three fields — a backing `Arc<dyn FileSystem>`, a `prefix`, and a `Rights` (`read`, `write`) — and every method funnels through one gate before touching the backing store (`crates/wanix-vfs/src/subtree.rs:80-133`):

```rust
fn require(&self, access: Access) -> FsResult<()> {
    let granted = match access {
        Access::Read => self.rights.read,
        Access::Write => self.rights.write,
    };
    if granted { Ok(()) } else { Err(FsError::PermissionDenied) }
}
```

Read operations require `Rights::read`; every mutation, plus opening a file for writing, requires `Rights::write` (`crates/wanix-vfs/src/subtree.rs:168-175`). Bits are additive — granting write does not imply read. A read-only `SubtreeFs` returns `PermissionDenied` for `open(write)`, `create_dir`, `remove_file`, `rename`, `symlink`, `set_times`, and every other mutating method, and the backing store is never even consulted. There is one place to audit the policy, not one per method.

## Re-rooting rewrites the path string — which is the hole

The rebase is deliberately simple: join the prefix and the relative path into a new `NormalizedPath` (`crates/wanix-vfs/src/subtree.rs:141-149`). Because both halves are normalized — no `..`, no absolute components — the *joined string* is always inside the prefix.

But that is a string fact, not a filesystem fact. On a backing that follows symlinks against a shared store — `LocalFs` over a host directory, the future `RemoteFs` — an in-prefix path named `escape` can be a symlink whose target is `../../docs/secret.txt`. That target is still inside the *backing root*, so `LocalFs`'s own root-confinement is satisfied; the string `projects/foo/escape` is inside the *prefix*, so the rebase is satisfied. Yet following the link reads a file outside the grant. Re-rooting alone leaks.

## confine_to_prefix closes it

`SubtreeFs` therefore routes every symlink-dereferencing method through a second helper, `rebase_confined`, which rebases and then asks the backing to confine the result (`crates/wanix-vfs/src/subtree.rs:151-166`):

```rust
fn rebase_confined(&self, path: &NormalizedPath) -> FsResult<NormalizedPath> {
    let rebased = self.rebase(path)?;
    self.backing.confine_to_prefix(&self.prefix, &rebased)?;
    Ok(rebased)
}
```

`confine_to_prefix` is a `FileSystem` trait hook (`crates/wanix-fs/src/traits.rs:207-231`). It confirms that `path`, *after the backing's own symlink resolution*, still lies within the subtree rooted at `prefix`. The host-backed override does the real work: it canonicalizes both the prefix host directory and the target, and rejects any resolved target that does not `starts_with` the resolved prefix (`crates/wanix-fs/src/localfs.rs:94-96`, `crates/wanix-fs/src/localfs/paths.rs:27-40`):

```rust
match fs::canonicalize(self.raw_host_path(path)) {
    Ok(resolved) if resolved.starts_with(&prefix_resolved) => Ok(()),
    Ok(_) => Err(FsError::PermissionDenied),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
    Err(error) => Err(map_io_error(error)),
}
```

Note the `NotFound` arm: a broken-but-in-prefix symlink is *not* an escape — the caller's real operation surfaces its own `NotFound` afterward. Confinement only fires when a link resolves to something real and outside.

The methods that follow links are the confined ones: `open`, `metadata` (and `metadata_with_lookup` only when `follow_symlinks()`), `read_dir`, and the *source* of a `hard_link` — minting a second name for an out-of-prefix inode reached through an in-prefix link is exactly the escape `hard_link` must also refuse (`crates/wanix-vfs/src/subtree.rs:178-228`). `read_link` is *not* confined: it returns the raw target bytes opaquely and never follows them, so it cannot leak content (`crates/wanix-vfs/src/subtree.rs:211-214`). A `NoFollow` stat reports the link's own metadata and skips confinement too.

## Under the hood: where it lives, and why

`SubtreeFs` lives in `wanix-vfs`, not `wanix-id` (`crates/wanix-vfs/src/subtree.rs`). The trust-boundary crate decides *whether* a peer may attach and *what* prefix it gets — see [attach policy](/concepts/attach-policy) — but the object that enforces the grant is a plain `FileSystem` in the namespace layer. That placement is the point: a grant is not an ACL the server checks on every request; it is a re-rooted filesystem the peer is simply handed. Even code holding the `SubtreeFs` directly cannot mutate past its prefix or below its rights, because there is no privileged path around `require` and `rebase_confined`. This is the mechanism behind [capability is a bind](/concepts/capability-is-a-bind): the capability *is* the confined, rights-gated subtree, enforced identically over a local bind, TCP, or QUIC.

It also reuses machinery you already have. The prefix-join is the same shape as a [namespace bind](/concepts/namespace-binding)'s source subpath; the path discipline is [NormalizedPath](/concepts/normalizedpath); and the host-side canonicalization is the same [host, not ambient, authority](/concepts/host-not-ambient-authority) discipline `LocalFs` applies everywhere else. The genuinely new parts are just two: the rights gate and the symlink confinement.

## See also

- [Capability is a bind](/concepts/capability-is-a-bind) — the grant that a SubtreeFs realizes.
- [Attach policy](/concepts/attach-policy) — default-deny grants that decide which prefix and rights a peer gets.
- [NormalizedPath](/concepts/normalizedpath) — the path type that already forbids `..` before any rebase.
- [Namespace binding](/concepts/namespace-binding) — the prefix-join shape SubtreeFs reuses.
- [Host, not ambient, authority](/concepts/host-not-ambient-authority) — why the host-backed override canonicalizes.

## Status / honest limits

- **Confinement is per backing filesystem, not global.** `SubtreeFs` calls `backing.confine_to_prefix`, so a symlink-following backing that does not override the hook would leak. `LocalFs` overrides it; the trait default is a no-op (`crates/wanix-fs/src/traits.rs:229-231`).
- **The no-op on opaque backings is deliberate, not a gap.** `MemFs` stores link targets opaquely and never resolves them as backing paths, so it cannot suffer the escape; its `confine_to_prefix` is correctly `Ok(())`.
- **Rights gate access, not resources.** A `SubtreeFs` enforces read vs write and prefix containment. It imposes no CPU, memory, or rate limit, and exec devices (`#task`, `#agent`, `#cpu`) reachable through one stay local-trust only — confinement scopes *what files* a peer touches, not *how much compute* it spends.
