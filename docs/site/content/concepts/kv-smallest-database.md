---
title: "#kv as the Smallest Database"
slug: concepts/kv-smallest-database
pageType: concept
oneLiner: In-process BTreeMap state that the mesh, handler, and cockpit all treat as a file; durable backing is a capsule, not a blur.
audience: [visionary, developer]
tags: [kv, mesh, shipped, in-memory, caveat]
sourceRefs:
  - crates/wanix-kv/src/lib.rs:6-21
  - crates/wanix-kv/src/lib.rs:120-166
  - crates/wanix-kv/src/files.rs:55-76
  - docs/recipes/04-tiny-http-app-with-kv.md:186-211
seeAlso:
  - devices/kv
  - concepts/wanix-capsule
  - concepts/http-app-route
  - concepts/devices-import-for-free
prerequisites:
  - devices/kv
usedInFlows:
  - {flow: learn/http-app-with-kv, step: 3}
honestLimits:
  - "#kv is in-memory: every key lives only as long as the serve process. On restart the BTreeMap is empty unless you froze it into a capsule first."
  - "Durable and content-addressed backing is a declared follow-up, not a wired feature."
  - "Each standalone qjs invocation that does not mount the served namespace gets its own fresh KvDevice, so its counter resets every run."
canonicalCaveatFor: [kv-in-memory]
---

# #kv as the Smallest Database

In-process BTreeMap state that the mesh, handler, and cockpit all treat as a file; durable backing is a capsule, not a blur.

You want application state — a request counter, a feature flag, a cursor. Wanix's answer is not a database client and not a config file on disk. It is a key under `#kv`: one file you read, one file you write. The store behind those files is a single `BTreeMap<String, Vec<u8>>` guarded by a lock (`crates/wanix-kv/src/lib.rs:21`). That is the whole engine. It is the smallest thing that earns the phrase "real database inside Wanix" precisely because it is a filesystem, not a database — and that distinction is what lets the same key be read by a local handler, a remote mesh peer, and the browser cockpit without any of them learning a new protocol.

## Read-modify-write a counter

Here is the entire state layer of an HTTP handler. A request comes in, the handler reads the current count, increments, writes it back:

```sh
# #kv values are plain files. Read the current value...
cat '#kv/http-counter'          # -> 41
# ...write the next one back.
echo 42 > '#kv/http-counter'
cat '#kv/http-counter'          # -> 42
```

Nothing in the handler holds the number. The qjs handler that backs the shipped, loopback-only HTTP-app route at `/.wanix/app/<name>` does exactly this on every request — open `#kv/http-counter` read-only, parse, increment, open it write-only with truncate, write the new bytes — and prints the result. The handler process can exit between requests and the count still climbs, because the only place the value lives is the serve process's `KvDevice` map.

The byte semantics are deliberately boring. Opening for read takes a snapshot of the value bytes at open time (`KvReadFile`), so a concurrent overwrite can never tear an in-flight read. Opening for write returns a `KvWriteFile` that buffers your bytes and commits them wholesale into the map when the handle drops (`crates/wanix-kv/src/files.rs:55-76`). A write replaces the whole value; there is no append, no journal, no fsync. That is one locked map mutation, and it is over.

## Why not files under the app directory

The obvious alternative is to write the count to `apps/counter.count.txt` on the host. It looks simpler. It degrades in three ways (`docs/recipes/04-tiny-http-app-with-kv.md:186-211`).

**Write churn.** Every increment against a host file is an open/truncate/write/close that the operating system journals and that any watcher — like a workbench tailing `apps/` — turns into an inotify storm. The `#kv` increment is one `BTreeMap::insert` under a lock (`crates/wanix-kv/src/files.rs:70-76`). No fsync amplification, no filesystem syscall per write.

**Durability clarity.** A file under `apps/` *looks* durable but commits on whatever sync semantics the host filesystem happens to give you. `#kv` says the honest thing out loud — its own module doc calls it "an in-memory tier; durable and content-addressed backing is a follow-up" (`crates/wanix-kv/src/lib.rs:6-7`). You are never confused about whether your state survives a restart: it does not, unless you made it.

**Mesh transparency.** Because `KvDevice` is a plain `FileSystem` (`crates/wanix-kv/src/lib.rs:120`), a remote node that mounts a peer's `#kv` over QUIC reads the same value the local handler reads, by resolving the same file path. A bag of host files gets none of that for free. This is the payoff of the file abstraction — see [devices import for free](/concepts/devices-import-for-free).

## State lives only while serve lives

The honesty in the module doc is the whole shape of the device. The map is created empty (`crates/wanix-kv/src/lib.rs:61-65`) and lives for exactly as long as the serve process that bound it. Stop serve, and the keys are gone. There is no disk file, no WAL, no recovery.

Two consequences follow that surprise newcomers. First, a `wanix qjs script.js` invocation that does *not* mount the served namespace allocates its own fresh `KvDevice`, so a counter in that standalone process resets to its starting value every run — the state-sharing only happens for tasks that resolve into the same served namespace. Second, "make this durable" is never "add a flag to `#kv`." The map stays in-memory on purpose; durability is a separate, explicit act.

That act is freezing the interesting keys into a capsule. A `.wcap` capsule snapshots a Wanix world into a portable, content-addressed artifact you can load elsewhere (see [wanix capsule](/concepts/wanix-capsule)). When you want a counter to outlive a restart, you do not blur the `#kv` line — you capture the key into a capsule and reload it. The boundary between "live, fast, ephemeral" and "frozen, portable, durable" stays a bright line you crossed on purpose.

## See also

- [#kv device](/devices/kv) — the file layout, modes, and exact open/read/write/remove contract.
- [wanix capsule](/concepts/wanix-capsule) — freeze a world (including chosen `#kv` keys) into a portable `.wcap`.
- [HTTP app route](/concepts/http-app-route) — the loopback-only `/.wanix/app/<name>` route the counter handler runs under.
- [Devices import for free](/concepts/devices-import-for-free) — why a peer's `#kv` is just a file across the mesh.
- Recipe: [a tiny HTTP app with #kv](/recipes/04-tiny-http-app-with-kv) — build the counter end to end.

## Status / honest limits

- **In-memory only.** The store is a `BTreeMap` in the serve process (`crates/wanix-kv/src/lib.rs:21`, `61-65`). Keys exist only while that process runs; a restart starts empty. To persist, freeze chosen keys into a capsule — durability is not a `#kv` option.
- **Durable and content-addressed backing is a follow-up, not shipped.** The module doc states this directly (`crates/wanix-kv/src/lib.rs:6-7`); do not read "smallest database" as "persistent database."
- **Standalone tasks do not share state.** Only tasks resolving into a served namespace see the same map; a bare `wanix qjs` run gets its own fresh device and its own counter, which resets each run.
- **Whole-value writes.** A write replaces the entire value on close (`crates/wanix-kv/src/files.rs:70-76`); there is no append or partial-update mode.
