---
title: Per-Process Namespaces
slug: concepts/per-process-namespaces
pageType: concept
oneLiner: Each task carries its own private, rearrangeable view of the filesystem tree; a child clones the parent's view rather than sharing one global root.
audience: [newcomer, developer, visionary]
tags: [namespace, plan9, shipped, isolation, mesh]
sourceRefs:
  - crates/wanix-vfs/src/lib.rs:33-52
  - crates/wanix-task/src/table.rs:99-101
  - crates/wanix-task/src/table.rs:168-196
  - crates/wanix-9p/src/session.rs:31-69
  - docs/mesh-the-missing-half-of-9p.md:63-89
  - docs/rust-vs-go-wanix.md:50-56
seeAlso:
  - concepts/everything-is-a-file
  - concepts/namespace-binding
  - concepts/namespace-resolution
  - concepts/missing-half-of-9p
  - concepts/capability-is-a-bind
  - concepts/trust-boundary-gaps
prerequisites:
  - concepts/everything-is-a-file
usedInFlows: []
honestLimits:
  - "The plain 9P serve path resolves every fid through one shared P9Server.root; per-principal namespaces only appear when an attach policy is installed."
  - "Tauth is ENOSYS: there is no 9P auth handshake, so the served namespace is not gated by a per-user identity on the plain path."
canonicalCaveatFor: []
---

# Per-Process Namespaces

Each task carries its own private, rearrangeable view of the filesystem tree; a child clones the parent's view rather than sharing one global root.

"Everything is a file" is the well-known Plan 9 slogan, but it is the smaller idea. The deeper one — the one that makes Wanix isolation and network transparency fall out almost for free — is that **the namespace is per-process and you can rearrange it.** Two tasks can read the same path and see different files, because each one carries its own binding table. This page is about where that table lives, what it is made of, and how a new task gets one.

## Show: two tasks, one path, two answers

A namespace is just a mapping from a destination path to the filesystems bound there. Bind a key/value store at `data` in one task and a memory directory at `data` in another, and `cat data/x` reads two unrelated things depending on which task asks. Nobody coordinated a global root; there is no global root to coordinate. The "current directory" of a Wanix task is not a string — it is an assembled view, and the view is the process's to edit.

## Name: a Namespace is a FileSystem

The object that holds this view is `Namespace`, and the first thing to notice is that it is itself a `FileSystem` (`crates/wanix-vfs/src/lib.rs:33-52`):

```rust
#[derive(Clone, Default)]
pub struct Namespace {
    bindings: BTreeMap<NormalizedPath, Vec<BindTarget>>,
}

impl FileSystem for Namespace { /* open, metadata, read_dir, ... */ }
```

That is the whole state: a `BTreeMap` from a destination `NormalizedPath` to a *vector* of bind targets. A vector, not a single target, because Plan 9 namespaces union: several filesystems can answer at one path, and resolution walks them in bind order until one says "found." `Namespace` satisfies the same `FileSystem` trait as `#kv`, a `MemFs`, or a `RemoteFs` (see [everything is a file](/concepts/everything-is-a-file)), so a namespace can be bound *inside another namespace*. Composition is closed under the trait — which is exactly why the same machinery works for a local directory and a remote peer.

`open` shows the union logic plainly: it asks each candidate target in turn and returns the first non-`NotFound` answer, tracking whether any candidate was a directory so a union of directories still reports `IsDirectory` rather than `NotFound` (`crates/wanix-vfs/src/lib.rs:54-68`). The binding mechanics — bind order, `First`/`Replace`/`Last` placement, subpath resolution — get their own pages: [namespace binding](/concepts/namespace-binding) and [namespace resolution](/concepts/namespace-resolution).

## Per-process: the view is `Clone`

`Namespace` derives `Clone`. That single line is what makes the view *per-process*. When Wanix spawns a child task, it does not hand the child a reference to a shared root; it clones the parent's binding table and gives the child its own copy.

The task table is where this happens (`crates/wanix-task/src/table.rs:99-101`):

```rust
/// Allocates a child task by cloning the parent's namespace.
pub fn allocate_child_of(&self, kind: impl AsRef<str>, parent: &Task) -> FsResult<Task> {
    self.allocate_task(kind.as_ref(), Some(parent.id()), parent.namespace())
}
```

`parent.namespace()` returns a clone of the parent's `Namespace`, and that clone becomes the child's. From that moment the two are independent: the child can bind, unbind, and reorder its view, and the parent never sees it. This is Plan 9's `fork`/`rfork` namespace semantics — a child inherits the *shape* of the parent's world and is then free to remodel its own copy. The Go Wanix made the same call: "process environment is not just env vars and cwd, but a namespace assembled from file services" (`docs/rust-vs-go-wanix.md:50-56`), and the Rust port keeps the binding model deliberately Plan 9-inspired.

## Why this is the real source of isolation and transparency

Once each process owns its view, two large properties stop being features you build and become things you *get*.

**Isolation by construction.** A task can only reach what its namespace binds. If you never bind `#cas` into a task, that task has no path to content-addressed storage — not because a permission check denied it, but because the name does not resolve to anything. Confinement is the *absence* of a bind, not an access-control list layered on top. (A capability, in Wanix, *is* a bind; see [capability is a bind](/concepts/capability-is-a-bind).)

**Network transparency for free.** The mesh's import half, `RemoteFs`, is also a `FileSystem`. Bind it at `/n/<peer>` and every resolution into that subtree becomes a 9P exchange on the wire — but the *calling* task sees only a path. Because the namespace is the universal join point and a remote service is just another `FileSystem` bound into it, you write the 9P client once and "every file-shaped service any node exports becomes reachable" (`docs/mesh-the-missing-half-of-9p.md:63-89`). The terminal, the task table, `#kv`, the agent — they all ride the same import. See [the missing half of 9P](/concepts/missing-half-of-9p).

## Under the hood: where the auto-bound `#task` view lands

A new task is never born with an empty world. When the table allocates *any* task — root or child — it binds that task's own `#task` view into the new namespace before handing it back (`crates/wanix-task/src/table.rs:168-196`):

```rust
let mut namespace = namespace;
namespace.bind(
    Arc::new(self.filesystem_for(id)),  // this task's #task view
    ".",
    "#task",
    BindOptions { position: BindPosition::Replace },
)?;
```

`filesystem_for(id)` produces a `TaskFs` scoped to *this* task id, so inside the task `#task/self/...` resolves to its own id, kind, cmd, env, dir, and exit files. The bind lands at destination `.` under the source name `#task` with `Replace`, so each task's `#task` is its own — a child sees *its* identity at `#task/self`, not its parent's, even though the binding table was cloned from the parent. The task is handed a world that already contains itself. The `#task` device contract is detailed on [the task device](/devices/task).

## See also

- [Everything is a file](/concepts/everything-is-a-file) — the trait that lets a namespace bind anything, including another namespace.
- [Namespace binding](/concepts/namespace-binding) — bind order, `First`/`Replace`/`Last`, and union placement.
- [Namespace resolution](/concepts/namespace-resolution) — how a path walks the binding table to a backing filesystem.
- [The missing half of 9P](/concepts/missing-half-of-9p) — import/export and `/n/`, the network payoff of a rearrangeable view.
- [A capability is a bind](/concepts/capability-is-a-bind) — confinement as the absence of a bind, not an ACL.
- [Trust boundary gaps](/concepts/trust-boundary-gaps) — what the per-principal namespace seam does not yet enforce.
- [The task device](/devices/task) — the `#task` view that lands in every new namespace.

## Status / honest limits

The per-process model is real inside the runtime: every task gets a cloned, independently editable `Namespace`. The boundary worth stating flatly is at the served edge.

- **The plain serve path shares one root.** On the plain 9P server path, `handle_attach` decodes the attach's `aname`/`uname` and then resolves *every* fid through a single shared `P9Server.root`; a per-principal namespace only appears when an `AttachPolicy` is installed, in which case `policy.evaluate(peer, aname)` selects a scoped root and a denial replies `EACCES` (`crates/wanix-9p/src/session.rs:31-69`). Without a policy, all attaches see the same namespace.
- **Tauth is ENOSYS.** There is no 9P auth handshake — `handle_auth` returns `ENOSYS` (`crates/wanix-9p/src/session.rs:71-74`). So the served namespace is not gated by a negotiated per-user identity on the plain path; identity-scoped namespaces live behind the policy seam, not the auth handshake.
- **Per-peer mount slots are a convention, not yet shipped.** The shipped CLI mount binds a single `/n/remote` slot, not a per-peer `/n/<peer-id>`; treat `/n/<peer>` as a labelled convention until per-peer slots ship (see [trust boundary gaps](/concepts/trust-boundary-gaps)).
