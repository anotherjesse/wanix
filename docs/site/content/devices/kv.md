---
title: "#kv — Key/Value Store"
slug: devices/kv
pageType: device
oneLiner: "#kv/<key> is a file: read returns the value (snapshot-on-open), write commits the whole buffer on close, listing #kv enumerates keys."
audience: [developer]
tags: [device, kv, service-device, shipped, mesh, caveat]
sourceRefs:
  - crates/wanix-kv/src/lib.rs:1-177
  - crates/wanix-kv/src/files.rs:7-76
  - docs/recipes/04-tiny-http-app-with-kv.md:186-211
seeAlso:
  - concepts/kv-smallest-database
  - concepts/blocking-stream-eof-contract
  - concepts/devices-import-for-free
  - concepts/wanix-capsule
  - devices/index
prerequisites:
  - concepts/service-devices
  - concepts/the-filesystem-trait
usedInFlows:
  - {flow: learn/http-app-with-kv, step: 3}
honestLimits:
  - "#kv is an in-memory Arc<RwLock<BTreeMap>>; values do not survive a serve restart — freeze the interesting keys into a capsule to persist them."
  - "A write commits the whole buffer wholesale on close; there is no partial update, append, or compare-and-set primitive."
  - "A reader sees a snapshot taken at open, so a value that changes mid-read is not observed by an in-flight read."
canonicalCaveatFor: []
---

# #kv — Key/Value Store

`#kv/<key>` is a file: read returns the value (snapshot-on-open), write commits the whole buffer on close, listing `#kv` enumerates keys.

`#kv` is the smallest "real database inside Wanix" — and it earns that name by *not* being a database. It is a plain `FileSystem` (`crates/wanix-kv/src/lib.rs:120`) where each key is a one-segment file. You read a key the way you read any file, you set it by writing and closing, you delete it by removing the file, and you list keys by reading the directory. Because it is a filesystem and nothing more, the same `KvDevice` that a local HTTP handler treats as a file, the mesh treats as a file — a remote node mounts `/n/<peer>/#kv/<key>` over QUIC and reads the same value, with zero device-specific networking code. This page is the contract: the four operations, the path rule, the modes, and the one boundary you must not blur.

## Read: `#kv/<key>` RDONLY → snapshot at open

Open a key read-only and you get a `KvReadFile` whose bytes were captured the instant you opened it (`crates/wanix-kv/src/lib.rs:129-131`, `crates/wanix-kv/src/files.rs:9-33`). `KvDevice::snapshot` takes the read lock, clones the stored `Vec<u8>`, and hands the handle that copy:

```sh
# a value previously committed under the key "counter"
wanix ...   # (#kv is bound by serve --wanix-services; see below)
```

The handle then serves those bytes from a private offset. The point of snapshot-on-open (`files.rs:7-8`) is tear-free reads: a concurrent overwrite cannot rewrite the buffer underneath an in-flight read, because the reader already owns its own copy. The flip side is the honest limit — if the value changes after you open, your read does not see the new bytes. Open again to observe the new value. A missing key returns `FsError::NotFound` at open (`lib.rs:76`), not an empty read.

## Write: `#kv/<key>` WRONLY|CREAT|TRUNC → buffer, commit on close

Open with any of `write`, `create`, or `truncate` and `open` returns a `KvWriteFile` (`lib.rs:125-128`). Writes accumulate into an in-handle buffer (`files.rs:60-63`); nothing touches the shared map until the handle is dropped. The `Drop` impl takes the write lock once and `insert`s the whole buffer wholesale, replacing any previous value (`files.rs:70-76`):

```rust
impl Drop for KvWriteFile {
    fn drop(&mut self) {
        if let Ok(mut map) = self.store.write() {
            map.insert(self.key.clone(), mem::take(&mut self.buffer));
        }
    }
}
```

This is buffer-and-commit-on-close: the value the key holds after you close is exactly the bytes you wrote, as one atomic replacement under a single lock. There is no append, no partial in-place update, and no compare-and-set — a write replaces the value, full stop. That is the entire mutation surface, and it is deliberately small. The single locked `BTreeMap::insert` is also why `#kv` is so much cheaper than writing state to a host file: no open/truncate/write/close churn, no fsync amplification, no inotify storm if the cockpit is watching the directory (`docs/recipes/04-tiny-http-app-with-kv.md:191-195`).

## `ensure_key`: the key exists at open, not just at close

A subtlety the 9P `Tlcreate` flow forces: a client opens a key for writing and then immediately stats the path, *before* the close-time commit. If the key only sprang into existence on close, that stat would fail. So `open` calls `ensure_key` first (`lib.rs:93-100`, called at `lib.rs:126`), which takes the write lock and does `store.entry(key).or_default()` — creating an empty value if the key is absent. After that, a stat between open and close finds a zero-length file, and the real value lands when the handle drops. This is why creating a key and writing nothing leaves an empty value behind rather than no key at all.

## List and remove: `#kv` is a directory of keys

Reading the directory (`read_dir` on the root, `lib.rs:142-154`) takes the read lock and returns one `DirEntry` per key, each carrying the value's current length as its size. That is how you enumerate everything `#kv` holds — including, on the served counter demo, the live `#kv/http-counter` key behind the HTTP app. Removing a key is `remove_file` on a single segment (`lib.rs:156-165`): it takes the write lock and `BTreeMap::remove`s the entry, returning `NotFound` if the key was never there.

## Path rule and modes

The path grammar is strict and total (`parse_path`, `lib.rs:109-118`). The normalized path `.` is the root directory; any path containing a `/` returns `FsError::NotFound`; everything else is a single-segment key. There is no nesting — `#kv` is flat by construction, so `#kv/a/b` is not "key `a` containing `b`," it is simply not found. Metadata follows the same split (`lib.rs:135-140`): the root reports `FileType::Directory` at mode `0o555` (`modes::DIRECTORY`), and a key reports `FileType::File` at mode `0o666` (`modes::VALUE_FILE`) with the value's byte length as its size (`lib.rs:168-174`, `26-29`). Read-write file, read-execute directory, no symlinks, no extended attributes.

## See also

- [#kv is the smallest database](/concepts/kv-smallest-database) — why "it is a filesystem, not a database" is the feature, not a shortcut.
- [Service devices](/concepts/service-devices) — the `#name` device pattern `#kv` is one instance of.
- [The FileSystem trait](/concepts/the-filesystem-trait) — the four methods `KvDevice` implements.
- [Devices import for free](/concepts/devices-import-for-free) — why a remote `/n/<peer>/#kv/<key>` reads the same value with no extra code.
- [The blocking stream / EOF contract](/concepts/blocking-stream-eof-contract) — `#pipe`/`#plumb` stream semantics, the contrast to `#kv`'s snapshot/commit model.
- [Wanix capsule](/concepts/wanix-capsule) — the answer when you need `#kv` state to survive a restart.
- [HTTP app with #kv](/learn/http-app-with-kv) and [Recipe 04](/recipes/04-tiny-http-app-with-kv) — `#kv/http-counter` behind a loopback HTTP app.
- [Devices index](/devices/index) — the rest of the service device set.

## Status / honest limits

- **In-memory only.** The store is one `Arc<RwLock<BTreeMap<String, Vec<u8>>>>` (`crates/wanix-kv/src/lib.rs:21,61-65`). Values live exactly as long as the process that holds the `KvDevice`. On a `serve` restart the map is empty again — `lib.rs:7` says it outright: "an in-memory tier; durable and content-addressed backing is a follow-up." When you need durability across restarts, freeze the interesting keys into a [capsule](/concepts/wanix-capsule); do not blur the in-process line by pretending `#kv` persists.
- **Bound by services, not standalone.** `#kv` is bound into the namespace only by `serve --wanix-services` (and the agent exec-server), so a fresh `wanix qjs` with no serve allocates its own empty `KvDevice` per process (`docs/recipes/04-tiny-http-app-with-kv.md:180-184`). Two processes share state only when they share one serve's namespace.
- **Whole-value, last-writer-wins.** A write replaces the entire value atomically on close (`crates/wanix-kv/src/files.rs:70-76`); there is no append, partial update, or atomic increment. Concurrent writers race to commit, and the last `Drop` wins.
- **Snapshot reads.** A read serves bytes captured at open (`files.rs:7-33`); a value that changes mid-read is not seen until you reopen.
