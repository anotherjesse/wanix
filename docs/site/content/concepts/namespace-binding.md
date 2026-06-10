---
title: Namespace Binding (bind / mount)
slug: concepts/namespace-binding
pageType: concept
oneLiner: bind() splices any FileSystem into the namespace at a chosen path with first/replace/last ordering, giving Plan 9 union mounts.
audience: [developer]
tags: [namespace, vfs, plan9, shipped]
sourceRefs:
  - crates/wanix-vfs/src/binding.rs:60-103
  - crates/wanix-vfs/src/lib.rs:54-94
  - crates/wanix-vfs/src/resolution.rs:9-29
  - crates/wanix-vfs/src/readdir.rs:30-86
  - rust-walkthrough.md:141-178
seeAlso:
  - concepts/namespace-resolution
  - concepts/per-process-namespaces
  - concepts/import-export-and-n
  - concepts/capability-is-a-bind
prerequisites:
  - concepts/per-process-namespaces
usedInFlows: []
honestLimits:
  - Do not hold the namespace lock while calling into a bound filesystem; bind() and resolution both call into the backing FileSystem.
  - bind() verifies the source exists at bind time but does not re-check it later; a source that disappears surfaces as a resolution-time NotFound.
canonicalCaveatFor: []
---

# Namespace Binding (bind / mount)

`bind()` splices any `FileSystem` into the namespace at a chosen path with first/replace/last ordering, giving Plan 9 union mounts.

A namespace starts empty. Every path a task can reach is there because something was *bound* into it. Binding is the one verb that assembles a task's file view: it takes any value that implements `FileSystem` — a host directory, a service device, a remote peer — and attaches a subtree of it at a destination path. Because the thing you bind and the namespace you bind it into are both just `FileSystem`s, the same verb composes local storage, the `#kv` device, and a machine across the mesh with no special cases.

## The one verb and its three positions

Here is the call (`crates/wanix-vfs/src/binding.rs:60`):

```rust
ns.bind(
    fs,                 // Arc<dyn FileSystem>: anything file-shaped
    "input.txt",        // source: which path inside fs
    "host",             // destination: where it lands in the namespace
    BindOptions::default(),
)?;
```

Three things land at the keyboard. First, `bind` checks that the source actually exists by calling `filesystem.metadata(&source)?` *before* recording anything — a bind onto a missing source fails immediately rather than rotting into a confusing resolution error later. Second, the binding is stored under the destination path. Third, the `position` in `BindOptions` decides what happens when something is already bound there.

That position is `BindPosition`, and it has exactly three values (`crates/wanix-vfs/src/binding.rs:8`):

```rust
pub enum BindPosition {
    #[default] First,   // insert before existing bindings here
    Replace,            // clear existing bindings, then add this one
    Last,               // insert after existing bindings here
}
```

This is Plan 9's `bind` ordering verbatim. `First` (the default) means a new binding *shadows* what was there for name lookups; `Last` means it falls *behind* it; `Replace` clears the slot. Where the names collide, order decides the winner — that is the whole mechanism behind union mounts.

## Union mounting: many filesystems, one path

A destination is not a single slot. Internally the namespace is a `BTreeMap<NormalizedPath, Vec<BindTarget>>` (`crates/wanix-vfs/src/lib.rs:35`): each destination owns a *list* of targets, and `position` chooses where in that list the new one goes. Bind two directories at the same destination and you get a union — listing it shows the files from both, and opening a name finds it in whichever target wins.

```sh
# Two host trees unioned at /lib, the second falling behind the first.
# /lib now lists the contents of both; on a name collision the First wins.
```

When you read the union directory, the VFS merges entries from every target and keeps the *first* occurrence of each name (`crates/wanix-vfs/src/readdir.rs:53`, an `or_insert_with`), so the bind order you chose is exactly the precedence you see. This is how a task gets a writable scratch dir layered over a read-only base, or how `#`-named devices coexist with your files at `/` — they are unioned, and the device names are simply hidden from the listing.

## Worked example: host dir, a device, a peer

The walkthrough mounts a real host directory into a task (`rust-walkthrough.md:141`):

```sh
mkdir -p /tmp/wanix-host
printf 'native mount' > /tmp/wanix-host/input.txt

wanix qjs --mount /tmp/wanix-host=host examples/qjs-host-mount.js
# host input: native mount
# host output: mounted output for native mount
```

`--mount /tmp/wanix-host=host` makes `wanix-cli` build a rooted `wanix-fs::LocalFs` over `/tmp/wanix-host` and `bind` it at the namespace path `host`. The task opens `host/input.txt` and writes `host/output.txt`; on the host those are `/tmp/wanix-host/...`. Host files are not ambient authority — the task only sees what was bound, and `LocalFs` guards the host root boundary (see [host, not ambient, authority](/concepts/host-not-ambient-authority)).

The same verb attaches the other two kinds of capability. A service device is a plain `FileSystem`, so binding `#kv` at `#kv` is the identical call — and once bound, `#kv/<key>` is just a file. A remote peer is also a `FileSystem`: the mesh's `RemoteFs` is bound at a `/n/<peer>` mount, and every resolution into that subtree becomes a 9P exchange on the wire (see [import, export, and /n](/concepts/import-export-and-n)). One verb, three radically different backings, no per-backing glue.

## Under the hood

`bind` records a `BindTarget { filesystem, source }` in the destination's list (`crates/wanix-vfs/src/binding.rs:71`). Resolution is the mirror image (`crates/wanix-vfs/src/resolution.rs:9`): for an incoming path it walks every destination, keeps the ones whose prefix the path falls under, rewrites the path to be relative to that target's source, and then sorts the candidates by `Reverse(destination_len)` — **longest matching destination first**. So a binding at `/a/b` always takes precedence over one at `/a` for a path under `/a/b`, and within one destination the bind order you set decides the rest.

The namespace's `FileSystem::open` then tries candidates in that order and returns the first that succeeds, treating `NotFound`/`NotDirectory` as "try the next target" (`crates/wanix-vfs/src/lib.rs:55-69`). That fall-through is what makes a union read like one tree: a name absent from the front binding is found in the one behind it. `bindings()` and `binding_count()` expose the stored table for inspection, in destination then resolution order (`crates/wanix-vfs/src/binding.rs:86-102`).

One contract that does *not* live in the position enum: `bind` re-roots, it does not constrain rights. Confining a bound subtree to a prefix or to read-only is a separate `SubtreeFs` wrapper — that is the subject of [a capability is a bind](/concepts/capability-is-a-bind).

## See also

- [Namespace resolution](/concepts/namespace-resolution) — how a path is matched against the bind table, longest-prefix first.
- [Per-process namespaces](/concepts/per-process-namespaces) — why each task carries its own bind table.
- [Import, export, and /n](/concepts/import-export-and-n) — binding a remote peer's namespace as files.
- [A capability is a bind](/concepts/capability-is-a-bind) — re-rooting and confining a bound subtree with `SubtreeFs`.
- [Host, not ambient, authority](/concepts/host-not-ambient-authority) — why a bound host dir is the only host the task sees.

## Status / honest limits

- **Do not hold the namespace lock while calling into a bound filesystem.** Both `bind` (which calls `metadata` on the source) and resolution (which calls `open`/`metadata`/`read_dir` on each target) reach into backing filesystems. Holding a namespace or filesystem lock across those calls invites deadlock; the codebase guardrail is explicit about this.
- **Source existence is checked once, at bind time.** `bind` verifies the source with `metadata` when you call it (`crates/wanix-vfs/src/binding.rs:69`); it does not pin or revalidate the source afterward. If the backing path later disappears, you see it as a resolution-time `NotFound`, not a bind error.
- **Union precedence is positional, not content-aware.** On a name collision the winner is whichever target comes first in resolution order — there is no merge, diff, or copy-up. The shadowed file is still reachable through a more specific bind, but a plain `open` returns only the front one.
