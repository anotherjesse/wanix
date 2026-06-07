---
title: NormalizedPath
slug: concepts/normalizedpath
pageType: concept
oneLiner: Wanix paths are relative, slash-separated, with no . or .. components and '.' as the root — the Go io/fs.ValidPath shape — so a path can never name something above where it starts.
audience: [developer]
tags: [filesystem, trust-boundary, shipped, caveat]
sourceRefs: [crates/wanix-fs/src/path.rs:5-78, crates/wanix-fs/src/error.rs:11-39, crates/wanix-cas/src/capsule.rs:14-24, crates/wanix-cas/src/capsule.rs:357-378, crates/wanix-vfs/src/subtree.rs:155-164]
seeAlso: [concepts/the-filesystem-trait, concepts/subtreefs-confine-to-prefix, concepts/capability-is-a-bind, concepts/wanix-capsule]
prerequisites: [concepts/the-filesystem-trait]
usedInFlows: []
honestLimits: ["Validating the path string blocks '..' but does not stop a symlink whose target escapes upward; that escape is caught separately by confine_to_prefix on symlink-following backings.", "'.' is the only path that bypasses validation, because it is constructed directly as the root.", "Normalization is shape validation, not canonicalization — it rejects bad paths, it does not collapse or rewrite them."]
canonicalCaveatFor: []
---

# NormalizedPath

Wanix paths are relative, slash-separated, with no `.` or `..` components and `.` as the root — the Go `io/fs.ValidPath` shape — so a path can never name something above where it starts.

Every method on the `FileSystem` trait takes a `NormalizedPath`, not a `&str`. That is a deliberate choke point. A path is validated *once*, at the boundary where it enters the filesystem layer, and from then on a device implementor handles a value that is guaranteed to be a safe, relative, well-formed path. There is no second place to re-litigate traversal safety, and no device that can quietly accept `../../etc/passwd` because its author forgot to check.

## The rule set

`NormalizedPath::new` accepts a string only if it passes one predicate (`crates/wanix-fs/src/path.rs:69-78`):

```rust
fn is_valid_path(path: &str) -> bool {
    if path == "." {
        return true;
    }
    if path.is_empty() || path.starts_with('/') || path.ends_with('/') {
        return false;
    }
    path.split('/')
        .all(|component| !component.is_empty() && component != "." && component != "..")
}
```

So the valid paths are exactly: the literal `.`, or a non-empty string that is relative (no leading `/`), has no trailing `/`, and whose every `/`-separated component is non-empty and is neither `.` nor `..`. Anything else returns `FsError::InvalidPath` carrying the offending string (`crates/wanix-fs/src/error.rs:11`, `:39`). The crate's own test pins both halves: `".", "#task", "dir/file", "a-b/c_d"` are accepted; `"", "/", "/abs", "dir/", "dir//file", "dir/.", "../x", "x/../y"` are all rejected (`crates/wanix-fs/src/path.rs:85-105`).

This is the same shape Go's standard library settled on for `io/fs.ValidPath`, which is where the doc comment points (`crates/wanix-fs/src/path.rs:5`). Wanix did not invent a path dialect; it adopted a battle-tested one and made it a type.

## Why "no `..`" is a security property, not a limitation

Most path bugs are traversal bugs: a string arrives from a client, a peer, or a frozen world, and somewhere downstream a `..` walks out of the directory it was supposed to stay in. The usual defenses are scattered — a check here, a `realpath` there, a sanitizer that the next contributor forgets to call.

Wanix removes the entire class at the front door. A path that contains `..` is *not representable* as a `NormalizedPath`; the constructor refuses it. By the time a path reaches `open`, `read_dir`, or `rename`, there is no upward component left to interpret. A device receiving a `NormalizedPath` does not have to wonder whether the caller already validated it, because the type is the proof. The trust boundary moved from "every callsite remembers to check" to "the type cannot hold a bad value."

## `.` as the root, and how `#name` devices sit alongside

The root is the single string `.`. It is the one path that skips validation, because it is built directly rather than parsed: `NormalizedPath::root()` constructs `Self(".".to_owned())` (`crates/wanix-fs/src/path.rs:12-14`), and `parent()` returns `None` for it — the root has no parent (`:42-50`). Walking up a tree therefore terminates at `.` instead of climbing past it.

Service devices fit this shape without exception. `#task`, `#kv`, `#term`, and the rest are addressed by paths like `#task/self/id` or `#kv/greeting` — and `#task`, as a single component, passes `is_valid_path` cleanly (it is non-empty and not `.`/`..`). The `#` carries no special meaning *to the path validator*; it is an ordinary leading character. Device-ness is a namespace convention layered above, not a path rule. So the same `NormalizedPath` discipline that protects a directory tree also protects every device path, with no carve-out.

## Under the hood: where it gets re-applied

The payoff shows up most clearly where untrusted data crosses into the filesystem. Capsule materialization is the sharpest example. A `.wcap` capsule is a frozen world built from a manifest of `path -> blob-hash` entries; when you load and materialize one, the paths come from whatever the sender wrote. Materialization treats that as hostile by construction (`crates/wanix-cas/src/capsule.rs:14-24`): every manifest path is re-validated through `NormalizedPath` before anything is written.

The helper is small and deliberately reuses the same validator (`crates/wanix-cas/src/capsule.rs:359-378`):

```rust
fn ensure_safe_relative(path: &str) -> MaterializeResult<()> {
    NormalizedPath::new(path).map_err(|_| MaterializeError::UnsafePath(path.to_owned()))?;
    Ok(())
}
```

`safe_join` then walks the validated components onto the target directory and *double-checks* each one against `.`/`..`/empty before pushing it, so the function is safe even if called on raw input. A hostile manifest cannot write outside the target directory. The same validator backs `SubtreeFs` re-rooting, where a capability is expressed as a re-rooted subtree (a grant is a `SubtreeFs`, not an ACL): paths rebased into a granted prefix are normalized on the way in.

## See also

- [The FileSystem trait](/concepts/the-filesystem-trait) — every method takes a `NormalizedPath`; this is why.
- [SubtreeFs: confine to prefix](/concepts/subtreefs-confine-to-prefix) — the symlink-escape defense that path validation alone cannot provide.
- [A capability is a bind](/concepts/capability-is-a-bind) — a grant is a re-rooted `SubtreeFs`, and the re-rooting normalizes paths.
- [Wanix capsule](/concepts/wanix-capsule) — frozen worlds whose manifest paths are re-validated on materialize.

## Status / honest limits

- **String validation does not stop symlink escapes.** Rejecting `..` in the path string keeps the *path* inside its tree, but a path that names a symlink whose target points upward is a different attack. That is caught separately: `SubtreeFs` rebases every symlink-following operation through `FileSystem::confine_to_prefix`, which rejects an escape with `FsError::PermissionDenied` on symlink-following backings and is a no-op on opaque-symlink backings like `MemFs` (`crates/wanix-vfs/src/subtree.rs:155-164`). `NormalizedPath` is one layer of the defense, not all of it.
- **`.` is the one unvalidated path.** It bypasses `is_valid_path` because it is constructed directly as the root (`crates/wanix-fs/src/path.rs:12-14`), not parsed from input.
- **It is shape validation, not canonicalization.** `NormalizedPath::new` *rejects* malformed or upward-reaching paths; it does not collapse, rewrite, or resolve them. There is no "clean this path up for me" behavior — a path is either already valid or it is an error.
