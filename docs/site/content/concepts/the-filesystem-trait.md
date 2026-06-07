---
title: The FileSystem Trait
slug: concepts/the-filesystem-trait
pageType: concept
oneLiner: One Rust trait (open/metadata/read_dir/mutations) is the universal currency of the whole system; implement it and you are a Wanix capability.
audience: [developer]
tags: [filesystem, extension-points, shipped, mesh, caveat]
sourceRefs:
  - crates/wanix-fs/src/traits.rs:66-150
  - crates/wanix-fs/src/traits.rs:152-334
  - crates/wanix-kv/src/lib.rs:1-177
  - crates/wanix-kv/src/files.rs:7-76
seeAlso:
  - concepts/everything-is-a-file
  - concepts/normalizedpath
  - concepts/service-devices
  - concepts/subtreefs-confine-to-prefix
  - concepts/content-addressed-data-plane
  - devices/kv
  - reference/filesystem-trait
prerequisites:
  - concepts/everything-is-a-file
usedInFlows:
  - {flow: learn/add-a-service-device, step: 2}
honestLimits:
  - "#kv is in-memory: a key set in one serve process is gone when it exits; freeze a world to a capsule to persist."
  - "Device-aware hooks (read_ready, content_hash, confine_to_prefix) default to the inert answer; a device opts in, it does not get them for free."
  - "Keep a FileSystem impl under 250-350 lines; #kv is the deliberately small template, not a ceiling."
canonicalCaveatFor: []
---

# The FileSystem Trait

One Rust trait (open/metadata/read_dir/mutations) is the universal currency of the whole system; implement it and you are a Wanix capability.

[Everything is a file](/concepts/everything-is-a-file) tells you *what* the system looks like from the keyboard. This page is the other side of that contract: *what you write* to make a new capability exist. The whole bargain is one trait in `crates/wanix-fs/src/traits.rs`. Implement `FileSystem` (and its companion `File`), and your storage engine, your task runner, your LLM session — whatever it is — becomes something a namespace can bind, 9P can serve, and the mesh can import, with no extra work on your part. There is no second interface to satisfy. This is the page Theo, an integrator, copies from.

## The trait shape: four verbs and a pile of defaults

The pair is small. `File` (`crates/wanix-fs/src/traits.rs:66-150`) is an open handle — `read`, `write`, `seek`, `metadata`. `FileSystem` (`crates/wanix-fs/src/traits.rs:152-334`) resolves and mutates paths:

```rust
pub trait FileSystem: Send + Sync {
    fn open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>>;
    fn metadata(&self, path: &NormalizedPath) -> FsResult<Metadata>;
    fn read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>>;
    // symlink, hard_link, create_dir, remove_file, remove_dir, rename,
    // set_permissions, set_times, content_hash, confine_to_prefix ...
}
```

Only three methods have no default: `open`, `metadata`, and `read_dir`. Everything else — every mutation, every link operation — ships with a body that returns `FsError::NotSupported` (`crates/wanix-fs/src/traits.rs:254-333`). That is the load-bearing design choice. A device implements the operations its capability *actually has* and inherits an honest error for the rest. A read-only device that never overrides `write` on its `File` (`crates/wanix-fs/src/traits.rs:79-81`) is telling the caller, truthfully, "you cannot write me." You never write a method just to throw `unimplemented!()`; the default already says the right thing.

## Walk `#kv`: the canonical minimal implementation

The smallest real `FileSystem` in the tree is `#kv`, the key/value device (`crates/wanix-kv/src/lib.rs:1-177`). At the keyboard it is plain file plumbing:

```sh
echo 'on' > '#kv/feature.flag'
cat '#kv/feature.flag'          # -> on
ls '#kv'                        # lists the keys
```

The whole implementation is roughly 170 lines, and three of them are the entire path model. `KvPath` is either `Root` or a single `Key`, and `parse_path` enforces that a key is one segment (`crates/wanix-kv/src/lib.rs:109-118`):

```rust
fn parse_path(path: &NormalizedPath) -> FsResult<KvPath<'_>> {
    let raw = path.as_str();
    if raw == "." { return Ok(KvPath::Root); }
    if raw.contains('/') { return Err(FsError::NotFound); }
    Ok(KvPath::Key(raw))
}
```

`open` (`crates/wanix-kv/src/lib.rs:121-133`) branches on `OpenOptions`. A write/create/truncate open eagerly creates the key (so a stat issued between open and close still finds it — the 9P `Tlcreate` flow stats immediately after open) and returns a `KvWriteFile`. A read open snapshots the current value into a `KvReadFile`. `metadata` reports a directory for `Root` and a value file for a `Key`; `read_dir` enumerates the map; `remove_file` deletes a key. Every other mutation in the trait is left at its `NotSupported` default — `#kv` has no directories to make, no links to follow, no renames. That is the template: parse the path to your own small address space, branch `open` on intent, implement the handful of methods your capability means, and stop.

### Snapshot-on-open, commit-on-close

The two `File` handles encode `#kv`'s read/write semantics (`crates/wanix-kv/src/files.rs:7-76`). `KvReadFile` copies the value bytes at open time and serves them from an offset, so a concurrent overwrite cannot tear an in-flight read — the reader sees a consistent snapshot of the moment it opened. `KvWriteFile` buffers writes and commits them *wholesale* in its `Drop` impl (`crates/wanix-kv/src/files.rs:70-76`): the new value replaces the key when the handle closes. A half-written value is never visible. These are device decisions, not framework rules — a different device could stream, append, or commit per-write. The trait gives you the open handle; you decide what read and write mean on it.

## What defaults to NotSupported

When you implement a `FileSystem`, you inherit a working "I don't do that" for: `symlink`, `read_link`, `hard_link`, `create_dir`, `remove_file`, `remove_dir`, `rename`, `set_permissions`, and `set_times` (`crates/wanix-fs/src/traits.rs:245-333`). On the `File` side, `write`, `seek`, `tell`, and `set_len` default to `NotSupported` too (`crates/wanix-fs/src/traits.rs:79-142`). `#kv` overrides exactly `open`/`metadata`/`read_dir`/`remove_file` on the filesystem and `read`/`write`/`metadata` on its handles. A purely read-only device overrides fewer still. The floor of a useful device is genuinely three methods plus one `File`.

## Device-aware hooks: opt in, or get the inert answer

Four hooks let a device be smarter, and all four default to the answer that costs nothing:

- **`read_ready` / `write_ready`** (`crates/wanix-fs/src/traits.rs:116-130`) default to `Ok(true)` — regular byte files are always ready, including at EOF. A device with queued input (a pipe that should not report ready while empty) overrides `read_ready` so a nonblocking caller is told the truth.
- **`is_seekable`** (`crates/wanix-fs/src/traits.rs:104-106`) defaults to `false`. `#kv`'s read file is seekable in spirit (it has an offset) but does not advertise it; a device backed by a real byte range sets this `true`.
- **`content_hash`** (`crates/wanix-fs/src/traits.rs:181-205`) defaults to `Ok(None)` — "read me the ordinary way." It is the *single* door from the 9P control plane to the [content-addressed data plane](/concepts/content-addressed-data-plane): a CAS-backed filesystem overrides it to return a BLAKE3 hash so a large file is fetched as a verified blob instead of crawled through `Tread` windows, and returns `None` while a file is open for write so no client fetches a torn snapshot.
- **`confine_to_prefix`** (`crates/wanix-fs/src/traits.rs:229-231`) defaults to `Ok(())`. A re-rooting export ([SubtreeFs](/concepts/subtreefs-confine-to-prefix)) calls it before following a symlink; a host-backed filesystem overrides it to canonicalize and reject targets that escape the prefix, while an opaque store like `MemFs` cannot leak and keeps the no-op.

The shape of all four is the same: the default is correct for the common case, and a device pays only for the semantics it actually has.

## See also

- [Everything is a file](/concepts/everything-is-a-file) — the prerequisite: why one file protocol reaches every capability.
- [NormalizedPath](/concepts/normalizedpath) — every trait method takes a normalized path, so you never re-litigate `..` traversal.
- [Service devices](/concepts/service-devices) — the catalog of `#`-named devices, each one a plain `FileSystem`.
- [`#kv` device](/devices/kv) — the worked example on this page, at device depth.
- [SubtreeFs / confine to prefix](/concepts/subtreefs-confine-to-prefix) — where `confine_to_prefix` is consulted.
- [Content-addressed data plane](/concepts/content-addressed-data-plane) — where `content_hash` leads.
- [FileSystem trait reference](/reference/filesystem-trait) — the full method list and error meanings.

## Status / honest limits

- **`#kv` is in-memory.** The store is an `Arc<RwLock<BTreeMap<...>>>` (`crates/wanix-kv/src/lib.rs:21,61-65`); keys live only as long as the `serve` process. The HTTP counter demo's `#kv/http-counter` resets when serve exits. To persist a world, freeze it to a [capsule](/concepts/wanix-capsule).
- **Device-aware hooks default inert.** `read_ready`/`write_ready` (`Ok(true)`), `is_seekable` (`false`), `content_hash` (`Ok(None)`), and `confine_to_prefix` (`Ok(())`) all return the cheapest correct-for-the-common-case answer. A device that needs real readiness, seeking, blob offload, or symlink confinement must override them — they are not granted for free.
- **Keep it small.** `#kv` is deliberately the minimal template, near the project's 250-line warn / 350-line hard module limit. A `FileSystem` impl that grows past that should be split before new feature work lands in it, per the code-quality guardrails.
