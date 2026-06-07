---
title: Namespace Resolution (longest-prefix, union)
slug: concepts/namespace-resolution
pageType: concept
oneLiner: A path resolves against all bindings whose destination is a prefix, most-specific binding first, merging directories into one synthesized view.
audience: [developer]
tags: [namespace, vfs, resolution, plan9, shipped]
sourceRefs:
  - crates/wanix-vfs/src/resolution.rs:10-29
  - crates/wanix-vfs/src/lib.rs:54-98
  - crates/wanix-vfs/src/readdir.rs:9-103
  - crates/wanix-vfs/src/path.rs:12-67
seeAlso:
  - concepts/namespace-binding
  - concepts/per-process-namespaces
  - concepts/the-filesystem-trait
  - concepts/service-devices
prerequisites:
  - concepts/namespace-binding
usedInFlows: []
honestLimits:
  - Resolution runs per operation; there is no cached merged tree, so every open/metadata/read_dir re-walks the bindings.
  - read_dir fans out to every overlapping backing filesystem, so listing cost scales with the number of bindings, not the directory size.
---

# Namespace Resolution (longest-prefix, union)

A path resolves against all bindings whose destination is a prefix, most-specific binding first, merging directories into one synthesized view.

[Namespace binding](/concepts/namespace-binding) is the write side: you attach a backing `FileSystem` at a destination path. This page is the read side. When a task opens `wanix:/config`, something has to decide *which* backing filesystem answers, and what happens when two of them both claim the same directory. That decision is namespace resolution, and in Wanix it is a small, deterministic walk: collect every binding whose destination is a prefix of the path, sort most-specific first, and take the first candidate that resolves. For directory listings, do not stop at the first — merge them all.

## Most-specific binding wins

Bind two filesystems, one at the root and one deeper:

```sh
# /  -> base   (a MemFs)
# /n -> remote (an imported peer)
```

Now open `/n/notes.txt`. Both bindings could plausibly own it: `/` is a prefix of every path, and `/n` is a prefix too. Resolution does not guess — it ranks. `resolve_candidates` walks the binding table, keeps every destination that is a prefix of the requested path, and records each candidate's `destination_len` (the byte length of the destination string). Then it sorts by that length, largest first (`crates/wanix-vfs/src/resolution.rs:10-29`):

```rust
candidates.sort_by_key(|candidate| Reverse(candidate.destination_len));
```

So `/n` (length 2) outranks `/` (length 1). The more specific binding is tried first. This is Plan 9's longest-prefix rule: the deepest mount that covers a path is the one that owns it, and shallower binds only get a turn if the deeper one says "not here." For each surviving candidate the resolver strips the destination prefix off the requested path and re-joins it onto the binding's source path (`relative_to_destination` and `join_paths`, `crates/wanix-vfs/src/path.rs:12-40`), so the backing filesystem sees a path rooted in *its* tree, not yours.

## Open, metadata, and mutations: first that resolves

Every method on `Namespace` is the same loop over those ranked candidates: try, and fall through on the two "wrong door" errors. For `open` (`crates/wanix-vfs/src/lib.rs:55-69`):

```rust
for target in self.resolve_candidates(path)? {
    match target.filesystem.open(&target.path, options) {
        Ok(file) => return Ok(file),
        Err(FsError::IsDirectory) => saw_directory = true,
        Err(FsError::NotFound | FsError::NotDirectory) => {}
        Err(err) => return Err(err),
    }
}
```

`NotFound` and `NotDirectory` mean "this backing filesystem does not have it" — keep looking down the rank. Any *other* error (a permission failure, an I/O error) is real and returns immediately; the resolver does not paper over it by trying a shallower binding. `metadata`, `create_dir`, `remove_file`, `remove_dir`, `set_permissions`, and `set_times` all follow this exact shape (`crates/wanix-vfs/src/lib.rs:71-233`): iterate candidates, take the first success, remember the most informative failure, and translate the union's emptiness into the right error at the end.

That tail matters. If no candidate resolved a regular file but a binding's *destination* lives below the path you opened — say you opened `/n` and only `/n/notes.txt` is bound — the path is a synthesized parent directory, so `open` returns `IsDirectory` rather than `NotFound` (`has_synthetic_children`, `crates/wanix-vfs/src/resolution.rs:31-35`). The namespace fabricates the intermediate directories that your binds imply, even when no backing filesystem has a real directory there.

Mutations that touch two paths at once — `rename`, `hard_link` — are stricter: both endpoints must resolve to the *same* backing filesystem (`same_backing_mutation_targets`, `crates/wanix-vfs/src/lib.rs:116-187`). The namespace will not silently move bytes across a bind boundary.

## read_dir merges, it does not pick

Listing is the one operation that must not stop at the first hit. If `/data` has a binding to one filesystem and another binding overlays it, `ls /data` should show the *union*. `read_namespace_dir` builds a `DirectoryView`, then fans out (`crates/wanix-vfs/src/readdir.rs:9-86`):

```rust
view.add_resolved_entries(self.resolve_candidates(path)?)?;
view.add_synthetic_entries(&self.bindings, path);
```

It adds entries from every overlapping backing filesystem, plus synthetic entries for any binding destination that is an immediate child of this directory. Two rules keep the merge honest. First, on a name collision the *first* candidate wins — entries are inserted with `or_insert_with`, and because candidates arrive most-specific-first, the deeper binding's file shadows the shallower one (`crates/wanix-vfs/src/readdir.rs:46-59`). Second, `#`-prefixed names are filtered out of the listing (`is_hidden`, `crates/wanix-vfs/src/readdir.rs:101-103`), so [service devices](/concepts/service-devices) stay addressable-but-not-advertised.

### Worked example

Bind `MemFs A` (containing `readme.md`, `shared.txt`) and `MemFs B` (containing `notes.txt`, `shared.txt`) both at `/work`:

```sh
ls /work
# -> notes.txt  readme.md  shared.txt
cat /work/shared.txt
# -> A's copy   (A bound most-specific-first / first; it shadows B)
```

`read_dir` visited both backings and unioned their names into one sorted, de-duplicated view (`entries` is a `BTreeMap`, so output is ordered and unique). `shared.txt` appears once, resolving to whichever binding ranks first. Neither A nor B knows the other exists; the union lives only in the namespace, only for as long as the call runs.

## Status / honest limits

- **Resolution is per operation; there is no cached merged tree.** Every `open`, `metadata`, and `read_dir` re-walks the binding table and re-sorts candidates (`crates/wanix-vfs/src/resolution.rs:10-29`). The synthesized union directory is computed on the fly and discarded; nothing is memoized between calls. This keeps binds cheap to add and change, and keeps imported (remote) filesystems honest — there is no stale snapshot to invalidate — but it means listing cost scales with the *number of overlapping bindings*, since `read_dir` calls into each backing filesystem in turn (`crates/wanix-vfs/src/readdir.rs:30-61`).
- **First-binding-wins on collisions is order-sensitive.** When two bindings share a destination, [bind order](/concepts/namespace-binding) (`BindPosition::First`/`Last`/`Replace`) decides which copy of a colliding name is visible. Resolution is deterministic, but only as deterministic as the bind order you chose.
- **Cross-binding moves are refused, not faked.** `rename` and `hard_link` require both endpoints in the same backing filesystem; the namespace does not implement copy-across-mounts.

## See also

- [Namespace binding](/concepts/namespace-binding) — the write side: how a destination gets a backing filesystem and a bind order.
- [Per-process namespaces](/concepts/per-process-namespaces) — why two tasks resolve the same path differently.
- [The FileSystem trait](/concepts/the-filesystem-trait) — the per-method contract every candidate backing filesystem implements.
- [Service devices](/concepts/service-devices) — the `#`-named devices that resolution keeps addressable but hides from listings.
