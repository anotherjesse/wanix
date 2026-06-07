---
title: The FileSystem / File Trait Reference
slug: reference/filesystem-trait
pageType: developer
oneLiner: "The trait at hand: method-by-method, the defaults, device-aware semantics, and which methods default to NotSupported."
audience: [developer]
tags: [reference, shipped, extension-point, core]
sourceRefs:
  - crates/wanix-fs/src/traits.rs:1-334
  - crates/wanix-fs/src/path.rs:5-78
  - crates/wanix-fs/src/metadata.rs:1-124
  - crates/wanix-fs/src/error.rs:1-55
  - crates/wanix-fs/src/content_hash.rs:1-29
seeAlso:
  - concepts/the-filesystem-trait
  - concepts/normalizedpath
  - concepts/content-addressed-data-plane
  - concepts/subtreefs-confine-to-prefix
  - reference/extension-points
prerequisites:
  - concepts/the-filesystem-trait
usedInFlows: []
honestLimits:
  - "Most mutation and link methods default to FsError::NotSupported; a device only gets a behaviour by overriding the method."
  - "content_hash and confine_to_prefix are opt-in hooks with safe no-op defaults; the default fs is not content-addressed and performs no symlink-escape check."
  - "Do not hold a namespace or filesystem lock while calling into another filesystem."
canonicalCaveatFor: []
---

# The FileSystem / File Trait Reference

The trait at hand: method-by-method, the defaults, device-aware semantics, and which methods default to `NotSupported`.

Everything in Wanix is a file, and `crates/wanix-fs/src/traits.rs` is where that promise is cashed out as a Rust contract. Two traits do the work: `FileSystem`, which a namespace, a task, or a WASI adapter calls to open paths and walk directories; and `File`, the open handle it hands back. The shape of the design is that **almost every method has a default**, and the default is honest — either a safe no-op or `Err(FsError::NotSupported)`. A `#kv` device, a `MemFs`, a host-directory filesystem, and a `RemoteFs` are all the same trait; they differ only in which methods they bother to override. This page walks the contract method by method so a new device author knows exactly the minimum to implement and the maximum they may decline. ([the FileSystem trait](/concepts/the-filesystem-trait) covers the *why*; this is the *what*.)

## FileSystem: the required core

`FileSystem: Send + Sync` (`traits.rs:153`) has exactly three methods with **no default** — implement these or you do not have a filesystem:

- `open(&self, path: &NormalizedPath, options: OpenOptions) -> FsResult<Box<dyn File>>` (`traits.rs:159`) — returns an open handle.
- `metadata(&self, path: &NormalizedPath) -> FsResult<Metadata>` (`traits.rs:166`) — stat one path.
- `read_dir(&self, path: &NormalizedPath) -> FsResult<Vec<DirEntry>>` (`traits.rs:238`) — list a directory.

Note `&self`, not `&mut self`: a `FileSystem` is shared and must be internally synchronized. Note also that every path is a `NormalizedPath` (see below), so an implementation never parses or validates a raw string — the type already guarantees a relative, slash-clean path with no `.` or `..`.

`metadata_with_lookup(&self, path, lookup: MetadataLookup) -> FsResult<Metadata>` (`traits.rs:173`) is the one method between required and optional: it has a default that **delegates to `metadata`**, ignoring the lookup mode. A filesystem that has symlinks overrides it to honour `MetadataLookup::NoFollow` (`lstat`) versus `FollowSymlink` (`stat`); a filesystem without symlinks leaves the default, and `lstat` and `stat` collapse to the same answer.

## File: the open handle

`File: Send` (`traits.rs:66`) has exactly **one** method with no default: `metadata(&self) -> FsResult<Metadata>` (`traits.rs:149`). Everything else has a default that lets a read-only or write-only handle stay terse:

- `read(&mut self, buf) -> FsResult<usize>` — also required in practice (`traits.rs:72`); it has no default body and must be implemented.
- `write(&mut self, buf) -> FsResult<usize>` — default `Err(NotSupported)` (`traits.rs:79`).
- `seek(&mut self, from: FileSeekFrom) -> FsResult<u64>` and `tell(&self) -> FsResult<u64>` — both default `Err(NotSupported)` (`traits.rs:89`, `:98`).
- `is_seekable(&self) -> bool` — default `false` (`traits.rs:104`). A streaming handle (a pipe, a `recv` file) stays `false`; a byte file overrides to `true` and implements `seek`/`tell`.
- `read_ready(&self) -> FsResult<bool>` — default `Ok(true)`, **including at EOF** (`traits.rs:116`). A device with a queued input channel overrides this so a non-blocking poll does not falsely report data while the queue is empty.
- `write_ready(&self) -> FsResult<bool>` — default `Ok(true)` (`traits.rs:128`); most Wanix files accept writes synchronously.
- `set_len(&mut self, len) -> FsResult<()>` — `ftruncate`-style, default `Err(NotSupported)` (`traits.rs:140`).

So the floor for a read-only device file is two methods: `File::read` and `File::metadata`. A write-only control file adds `File::write`. The readiness pair is what makes a device honest under non-blocking I/O without forcing every file to think about it.

## content_hash: the bulk-offload hook

`content_hash(&self, path) -> FsResult<Option<ContentHash>>` (`traits.rs:203`) defaults to `Ok(None)` and most filesystems leave it there. It is the *only* hook by which the 9P control plane offloads bulk file bytes to the content-addressed data plane: when a CAS-aware client learns a hash for a large file, it fetches the BLAKE3-verified blob peer-to-peer instead of dragging the bytes through the `msize`-bounded `Tread` window. `ContentHash` is a 32-byte BLAKE3 digest newtype (`content_hash.rs:29`).

The contract has teeth: `Ok(None)` means "resolvable but not content-addressed — read it the ordinary way," and is *not* an error; a real error means the path could not be resolved at all. A `CasFs` decorator overrides this to return the live hash, and **must** return `None` while the file is open for write, so a client never fetches a torn snapshot. The hash travels to the client out of band, never as bytes appended to a fixed-shape 9P response, because this codebase's `Rgetattr` decoder rejects trailing bytes. See [the content-addressed data plane](/concepts/content-addressed-data-plane).

## confine_to_prefix: the symlink-escape hook

`confine_to_prefix(&self, prefix, path) -> FsResult<()>` (`traits.rs:229`) defaults to a no-op `Ok(())`. It is the confinement check a re-rooting export (`SubtreeFs`) consults before following symlinks. Re-rooting only rewrites the path *string*, so a filesystem that resolves symlinks against a shared backing store — a host directory — could otherwise satisfy an in-prefix path whose link target escapes the prefix while still inside the backing root. A `MemFs` stores link targets opaquely and never resolves them as backing paths, so it cannot suffer that escape and the no-op default is correct. Host-backed filesystems override this to canonicalize the target and return `FsError::PermissionDenied` when it lands outside `prefix`. A missing `path` is not an escape — the caller runs the real operation afterward and surfaces its own `NotFound`. This is the hook that makes a [grant a re-rooted SubtreeFs](/concepts/subtreefs-confine-to-prefix) safe rather than a string trick.

## The methods that default to NotSupported

Everything else on `FileSystem` defaults to `Err(FsError::NotSupported)`, which is why a read-only device implements three methods and declines the rest by omission:

- `read_link(&self, path) -> FsResult<Vec<u8>>` (`traits.rs:245`) — returns the uninterpreted link target bytes.
- `symlink(&self, target, path)` (`traits.rs:254`) and `hard_link(&self, old, new)` (`traits.rs:265`).
- `create_dir`, `remove_file`, `remove_dir`, `rename` (`traits.rs:274`, `:283`, `:292`, `:301`).
- `set_times(&self, path, accessed_ns, modified_ns)` (`traits.rs:326`) — nanoseconds since the Unix epoch.

One method is subtly different: `set_permissions(&self, path, permissions)` (`traits.rs:313`) calls `self.metadata(path)?` first, then returns `NotSupported`. The effect is that a missing path reports `NotFound`, while an existing path on a filesystem that ignores mode bits reports `NotSupported` — the error reflects the real cause rather than masking a missing file.

## The supporting types

The trait's vocabulary is small and lives in the same crate:

- **`NormalizedPath`** (`path.rs:7`) — the `io/fs.ValidPath` shape from Go. `.` is the root; any other path is relative, non-empty, slash-separated, and free of `.` and `..`. `new` validates and returns `FsError::InvalidPath` on failure; `root()`, `parent()`, and `file_name()` are pure accessors. Construct it once at the boundary and the rest of the stack trusts it. See [NormalizedPath](/concepts/normalizedpath).
- **`OpenOptions`** (`traits.rs:33`) — four bool fields (`read`, `write`, `create`, `truncate`) with `OpenOptions::read()` and `::read_write()` constructors.
- **`MetadataLookup`** (`traits.rs:5`) — `FollowSymlink` vs `NoFollow`, with `follow_symlinks()`.
- **`FileSeekFrom`** (`traits.rs:22`) — `Start(u64)`, `Current(i64)`, `End(i64)`.
- **`Metadata`** (`metadata.rs:14`) — `file_type` (`File` / `Directory` / `Symlink`), `len`, `mode`, `link_count`, and three nanosecond timestamps (`accessed`, `modified`, `changed`). Built via `Metadata::new`, `new_with_times`, or `new_with_links`; all fields are read through accessors. `DirEntry` (`metadata.rs:128`) pairs a basename with `Metadata`.
- **`FsError`** (`error.rs:9`) — the closed error vocabulary: `InvalidPath`, `NotFound`, `NotSupported`, `PermissionDenied`, `AlreadyExists`, `NotDirectory`, `IsDirectory`, `InvalidFd`, `InvalidOffset`, `InvalidTime`, `NotEmpty`, and an `Other(String)` escape hatch. `FsResult<T>` aliases `Result<T, FsError>`. These variants map directly to 9P/errno at the server edge, so picking the right one matters.

## See also

- [The FileSystem trait](/concepts/the-filesystem-trait) — the conceptual "why everything is this one trait."
- [NormalizedPath](/concepts/normalizedpath) — the path type every method takes.
- [The content-addressed data plane](/concepts/content-addressed-data-plane) — where `content_hash` plugs in.
- [SubtreeFs / confine to prefix](/concepts/subtreefs-confine-to-prefix) — the consumer of `confine_to_prefix`.
- [Extension points](/reference/extension-points) — the full menu of traits you implement to extend the core.

## Status / honest limits

- **Most methods default to `NotSupported`.** A device gets a behaviour only by overriding the method; the trait does not infer capabilities. The required floor for a `FileSystem` is `open`, `metadata`, and `read_dir`, and for a `File` it is `read` and `metadata`. Everything else — write, seek, links, mutations, timestamps — must be opted into explicitly.
- **The two device-aware hooks ship safe no-op defaults.** `content_hash` returns `Ok(None)` (not content-addressed) and `confine_to_prefix` returns `Ok(())` (no symlink-escape check). A host-backed filesystem that does not override `confine_to_prefix` is relying on a no-op; this is correct for opaque-symlink stores like `MemFs` and unsafe for backing-resolved symlinks, so the override is mandatory there.
- **Concurrency rule from the guardrails.** Do not hold a namespace or filesystem lock while calling into another filesystem. Because composition (binds, imports, re-roots) means one `FileSystem` method routinely calls another, holding a lock across that call risks deadlock; release before you delegate.
