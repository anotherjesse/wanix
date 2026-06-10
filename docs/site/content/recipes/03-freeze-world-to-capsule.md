---
title: Recipe 03 — Freeze a Wanix World to a Portable Capsule
slug: recipes/03-freeze-world-to-capsule
pageType: use-case
oneLiner: Freeze a directory the agent (or you) just built onto the content-addressed plane so the same world can be reconstructed deterministically anywhere the same capsule id is reachable.
audience: [developer, visionary]
tags: [cli, mesh, shipped, caveat, content-addressed, capsule]
sourceRefs: [docs/recipes/03-freeze-world-to-capsule.md, crates/wanix-cli/src/capsule.rs, crates/wanix-cas/src/capsule.rs:244-301, crates/wanix-cas/src/capsule.rs:138-225]
seeAlso: [concepts/wanix-capsule, devices/cas, concepts/end-to-end-hash-verification, concepts/one-identity-two-planes, recipes/02-mount-remote-peer, use-cases/portable-worlds]
prerequisites: [concepts/wanix-capsule, devices/cas]
usedInFlows: [{flow: portable-worlds, step: 1}]
honestLimits:
  - There is no .wcap archive file in the shipped CLI; a capsule is the manifest blob plus its referenced file blobs sitting in the CAS store.
  - Live #kv state is not portable — #kv is a service device, not a directory under the world root; copy the values you want frozen into the world tree first.
  - Ephemeral handles (open #task/#term/#cas-ingest fds, in-flight 9P fids) are gone the moment the originating process exits; a capsule is a tree, not a running task.
  - Symlinks, special files, extended attributes, and permissions are not preserved; materialized files land as plain files.
canonicalCaveatFor: []
---

# Recipe 03 — Freeze a Wanix World to a Portable Capsule

Freeze a directory the agent (or you) just built onto the content-addressed plane so the same world can be reconstructed deterministically anywhere the same capsule id is reachable.

**What & why.** An agent ran, wrote some scripts, materialized some state, and produced a working directory. Now you want that exact tree to exist somewhere else — another account, another machine, a peer across the mesh — without zipping it, without trusting a mirror, without wondering whether the copy drifted. `wanix capsule save` walks the tree, ingests every file as a content-addressed blob, and ingests a sorted manifest of those blobs as one more blob. The hash of that manifest blob *is* the capsule id: a single 64-hex string. Share the string, share the world. `wanix capsule load <id> DIR` reconstructs the tree byte-for-byte, re-hashing every blob before it touches disk. This is [venti](/devices/cas) applied to a whole world — the durable, verifiable half of the mesh's two planes.

## 1. Set up a small world

Build a directory we will treat as one frozen world — a qjs program plus the data it reads:

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix='./target/debug/wanix'

mkdir -p /tmp/world-A/scripts /tmp/world-A/state
cat > /tmp/world-A/scripts/hello.js <<'JS'
import * as std from "qjs:std";
std.out.puts(std.loadFile("state/greeting.txt") || "hello\n");
JS
echo "hello, capsule" > /tmp/world-A/state/greeting.txt

# --cwd is a namespace path, so bind the host dir in with --mount first:
wanix qjs --mount /tmp/world-A=world --cwd world \
  /tmp/world-A/scripts/hello.js                      # -> hello, capsule
```

If your state lives in `#kv` rather than on disk, it is *not* in this tree yet. `#kv` is a [service device](/devices/kv), not a directory under the world root, so a capsule does not see it. Materialize the keys you care about as files first — capsules freeze *file content*, not live device state. That line is deliberate: a capsule is a deterministic on-disk view of a world; live mesh state stays out of scope (section 5).

## 2. Freeze with `wanix capsule save`

The CLI grammar, parsed in `crates/wanix-cli/src/capsule.rs:51`, is two verbs plus an optional store flag:

```text
wanix capsule (save DIR | load CAPSULE_ID DIR) [--store DIR]
```

`--store DIR` overrides the CAS store directory; the default is `$WANIX_CAS_DIR`, or the per-user owner-private cache when that is unset (`open_store`, `capsule.rs:131`). Pin it to a known path so the next step can look inside:

```sh
export CAS=/tmp/cas-A
wanix capsule save /tmp/world-A --store "$CAS"
# capsule 7f3a…<64 hex>… saved (2 files) from /tmp/world-A
# load with: wanix capsule load 7f3a… <DIR>
```

That output is built by `save` in `capsule.rs:138`: it calls `Capsule::freeze`, prints `capsule.id().to_hex()` and the file count. Capture the id — it is the only thing you ship:

```sh
CAPSULE_ID=$(wanix capsule save /tmp/world-A --store "$CAS" \
  | awk '/^capsule/ {print $2}')
```

Under the hood, `Capsule::freeze` (`crates/wanix-cas/src/capsule.rs:244`) recurses the tree (`collect_files`, `capsule.rs:305`), `put`s each file's bytes as a blob, builds a `BTreeMap` keyed by world-relative path so ordering is deterministic, serializes the sorted [`WorldManifest`](/concepts/wanix-capsule) to a blob, and `put`s *that* — its hash is the capsule id.

## 3. Inspect the CAS-backed structure

There is no archive, no per-capsule subdirectory. The store is one flat directory of blobs, each filename being the lowercase-hex BLAKE3 of its bytes:

```sh
ls "$CAS"
# 7f3a…   <- the manifest blob (its hash IS $CAPSULE_ID)
# a91b…   <- scripts/hello.js bytes
# 4c02…   <- state/greeting.txt bytes
```

The manifest blob is plain text — one `\n`-terminated `"<hex-hash> <path>"` line per file, sorted by path (`WorldManifest::to_blob`, `capsule.rs:144`):

```sh
cat "$CAS/$CAPSULE_ID"
# a91b…  scripts/hello.js
# 4c02…  state/greeting.txt
```

That exact byte form is what gets hashed into the capsule id, which is why sorting matters: the same input tree freezes to the same id on every host, and identical files across worlds dedup to one blob. The serializer must stay stable or every id moves.

## 4. Load the capsule elsewhere

"Elsewhere" is any place whose CAS store has — or can fetch — the manifest blob and every blob it names.

**Same machine, different directory.** The fastest proof:

```sh
wanix capsule load "$CAPSULE_ID" /tmp/world-B --store "$CAS"
# capsule 7f3a… restored (2 files, 38 bytes) to /tmp/world-B
diff -r /tmp/world-A /tmp/world-B   # no output -> byte-identical
```

`load` (`capsule.rs:151`) calls `Capsule::load` to fetch and parse the manifest, then `materialize` to write the tree. Both halves are defensive: `WorldManifest::from_blob` (`capsule.rs:163`) caps the manifest at `MAX_MANIFEST_ENTRIES` (100k) and `CAPSULE_MANIFEST_MAX_BYTES` (16 MiB), and re-validates every path through [`NormalizedPath`](/concepts/normalizedpath); `materialize` (`capsule.rs:202`) `safe_join`s each path under the target so a hostile manifest cannot escape it.

**Different machine — ship the blobs.** The portable form today is "the blobs the capsule references." `awk` the manifest for hashes, add the manifest blob itself, rsync them, then `load` against the destination store:

```sh
( echo "$CAPSULE_ID"; awk '{print $1}' "$CAS/$CAPSULE_ID" ) \
  | xargs -I{} echo "$CAS/{}" > /tmp/capsule-files.txt
rsync -av --files-from=/tmp/capsule-files.txt / hostB:/tmp/cas-B/
ssh hostB wanix capsule load "$CAPSULE_ID" /tmp/world-B --store /tmp/cas-B
```

**Over the mesh.** The async, network-fetching `IrohCasStore` implements the same `ContentStore` trait, so the same `capsule load <id>` resolves blobs through a peer's published ticket. The local trust boundary still runs on materialize: every `get` re-hashes bytes against their content address ([end-to-end hash verification](/concepts/end-to-end-hash-verification)), so a tampered or hostile remote blob is rejected, not written.

## 5. What is portable, what is not

Portable, because the capsule guarantees it: **file content** under the world root; **determinism** (sorted manifest, same id every time, cross-world dedup); **integrity** (every blob re-hashed before any byte hits disk).

Not portable — do not expect a capsule to carry these:

- **Live `#kv` state.** `#kv` is in-memory and lives only as long as the serve process; it is a service device, not a tree under the world root. Copy the values you want into the world first (section 1).
- **Ephemeral fds and handles.** `#task` exec sessions, open `#term/<id>` resources, in-flight 9P fids, `#cas/ingest` write handles — all gone when the originating process exits. To carry a *running* program, that is the separate QuickJS snapshot/resume mechanism, not a capsule.
- **Live mesh peers and tickets.** Resolving a capsule id over the network needs the peer relationship and iroh ticket — runtime state, not part of the capsule.
- **Symlinks, special files, xattrs, permissions.** Symlinks are skipped on freeze (`collect_files`, `capsule.rs:318`) because a capsule freezes content, not link topology; materialized files are plain files. Layer your own metadata file in if you need more.

## 6. Try the round-trip yourself

A minimal smoke test:

```sh
export CAS=$(mktemp -d); SRC=$(mktemp -d); DST=$(mktemp -d)
echo "hello, capsule" > "$SRC/greeting.txt"
ID=$(wanix capsule save "$SRC" --store "$CAS" | awk '/^capsule/ {print $2}')
wanix capsule load "$ID" "$DST" --store "$CAS"
diff -r "$SRC" "$DST" && echo "round-trip ok: $ID"
```

That id is the only durable thing you need to reconstruct the world. It is the local-store form of the mesh's content-addressed data plane — the half of [one identity, two planes](/concepts/one-identity-two-planes) that you can freeze, hand off, and verify on arrival.

## See also

- **Concepts:** [Wanix capsule](/concepts/wanix-capsule) · [End-to-end hash verification](/concepts/end-to-end-hash-verification) · [One identity, two planes](/concepts/one-identity-two-planes)
- **Devices:** [`#cas`](/devices/cas) · [`#kv`](/devices/kv)
- **Recipes & use cases:** [Recipe 02 — Mount a remote peer](/recipes/02-mount-remote-peer) · [Portable worlds](/use-cases/portable-worlds) · [Your personal compute mesh](/use-cases/personal-compute-mesh)

## Status / honest limits

- **No `.wcap` archive file in the shipped CLI.** A capsule is the manifest blob plus its referenced file blobs in the CAS store. The single transportable seed is `$CAS/<capsule-id>` (the manifest); ship it plus the blobs it names, or fetch over the mesh.
- **`#kv` state is not in the capsule.** `#kv` is in-memory and bound to the serve process. Materialize the keys you want into the world tree before freezing.
- **A capsule is a tree, not a task.** Ephemeral fds, exec sessions, terminal resources, and process identity/env/cwd are not carried. Re-launch policy is the caller's.
- **No symlinks, special files, xattrs, or permissions.** Materialized files are plain files; symlinks are skipped on freeze to avoid escaping the world root.
- **Mesh fetch depends on runtime state.** A capsule id is a content address; resolving it over iroh needs the peer relationship and ticket, which the capsule does not contain.
