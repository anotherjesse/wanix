# Recipe 03 — Freeze a Wanix world to a portable capsule

Goal: take a directory the agent (or you) just built — qjs scripts, data in
`#kv`, whatever — and freeze it onto the content-addressed plane so the same
world can be reconstructed deterministically anywhere the same capsule id is
reachable.

A capsule is venti applied to a whole "world" (a directory tree):

- Every file becomes a blob in a [`ContentStore`][store]. Identical bytes dedup
  to one entry.
- The deterministic sorted [`WorldManifest`][capsule] (one
  `"<hex-hash> <path>"` line per file, paths sorted) is itself stored as a blob.
- *That manifest blob's hash is the capsule id.* Share the id, share the world.

There is no special `.wcap` archive file in the shipped CLI today — the capsule
is the manifest blob plus its referenced file blobs, sitting in the CAS store.
If you want a single transportable artifact, the file at
`$WANIX_CAS_DIR/<capsule-id-hex>` (the manifest blob) is the seed; copy it plus
the referenced blobs, or use [`IrohCasStore`][lib] over the mesh.

[store]: ../../crates/wanix-cas/src/store.rs
[capsule]: ../../crates/wanix-cas/src/capsule.rs
[lib]: ../../crates/wanix-cas/src/lib.rs

## 1. Set up a small world

Build a directory we will treat as one frozen world. We will drop in a qjs
program and some data that mimics state an agent might have written under
`#kv`.

```sh
mkdir -p /tmp/world-A/scripts /tmp/world-A/state
cat > /tmp/world-A/scripts/hello.js <<'JS'
import * as std from "qjs:std";
const greeting = std.loadFile("state/greeting.txt") || "hello\n";
std.out.puts(greeting);
JS

cat > /tmp/world-A/state/greeting.txt <<'TXT'
hello, capsule
TXT

# (Optional) run it to confirm the world works before freezing.
wanix qjs --cwd /tmp/world-A scripts/hello.js
```

If you have been driving `#kv` from a running agent, that state lives in the
agent's local KV directory, not inside the world directory. To make it part of
the capsule, materialize the keys you care about as files first — capsules
freeze *file content*, not live device state. A common pattern:

```sh
# Copy the values you want frozen out of the live KV root into the world dir.
cp -R "$WANIX_KV_DIR/myproject" /tmp/world-A/state/kv/
```

That is the trust-boundary line: a capsule represents a deterministic on-disk
view of a world; live mesh state (peers, leases, ephemeral handles) is
explicitly out of scope. More on this in section 5.

## 2. Freeze with `wanix capsule save`

The CLI is in [`crates/wanix-cli/src/capsule.rs`][cli]. The grammar (from
[`help.rs`][help]) is:

```text
wanix capsule (save DIR | load CAPSULE_ID DIR) [--store DIR]
```

Flags, parsed in [`parse_capsule_command`][cli]:

- `save DIR` — freeze the tree at `DIR` into the CAS store.
- `load CAPSULE_ID DIR` — fetch the manifest blob by its 64-hex hash, fetch
  every referenced file blob, write the tree out under `DIR`.
- `--store DIR` — override the CAS store directory. Default is
  `$WANIX_CAS_DIR` (see [`CAS_DIR_ENV`][local]) or, when unset, the per-user
  owner-private cache (`cas-store/` under the platform cache root).

[cli]: ../../crates/wanix-cli/src/capsule.rs
[help]: ../../crates/wanix-cli/src/help.rs
[local]: ../../crates/wanix-cas/src/local.rs

Freeze the world we just built. We will pin the store to a known directory so
the next steps can inspect it.

```sh
export CAS=/tmp/cas-A
wanix capsule save /tmp/world-A --store "$CAS"
```

Output (the capsule id and a copy-pasteable load command):

```text
capsule 7f3a…<64 hex chars>… saved (2 files) from /tmp/world-A
load with: wanix capsule load 7f3a… <DIR>
```

The id is what you share. Save it:

```sh
CAPSULE_ID=$(wanix capsule save /tmp/world-A --store "$CAS" \
  | awk '/^capsule/ {print $2}')
echo "$CAPSULE_ID"
```

## 3. Inspect the CAS-backed structure

A `LocalCasStore` (see [`local.rs`][local]) is one flat directory of
blobs, each filename being the lowercase-hex BLAKE3 of its bytes. There is no
per-capsule subdirectory: blobs from every capsule pool together, which is what
makes dedup work across worlds.

```sh
ls "$CAS"
# 7f3a…   <- the manifest blob (its hash IS $CAPSULE_ID)
# a91b…   <- scripts/hello.js bytes
# 4c02…   <- state/greeting.txt bytes
```

The manifest blob itself is plain text:

```sh
cat "$CAS/$CAPSULE_ID"
# a91b…b7  scripts/hello.js
# 4c02…d3  state/greeting.txt
```

One `\n`-terminated `"<hex-hash> <path>"` line per entry, entries sorted by
path — that exact byte form is hashed to produce the capsule id, so it must
stay stable. The serializer is [`WorldManifest::to_blob`][capsule].

Hostile-manifest defenses live in the same module:

- [`MAX_MANIFEST_ENTRIES`][capsule] (100k) caps fan-out — a manifest with
  millions of entries is rejected before any file write.
- [`CAPSULE_MANIFEST_MAX_BYTES`][capsule] (16 MiB) caps the manifest blob itself.
- [`MAX_BLOB_SIZE`][store] (256 MiB) caps any single file blob.
- Every path is re-validated through `wanix_fs::NormalizedPath`
  ([`ensure_safe_relative`][capsule]) — no `..`, no absolute, no escape.
- Every store `get` rehashes bytes before returning them
  ([`verify_hash`][store]), so a tampered file under `$CAS/<hex>` is rejected
  with `HashMismatch` rather than served under the wrong content address.

## 4. Load the capsule elsewhere

"Elsewhere" can be another machine, another user account, or just a different
target directory — anything where the CAS store contains (or can fetch) the
manifest blob and every blob it references.

### Same machine, different directory

```sh
wanix capsule load "$CAPSULE_ID" /tmp/world-B --store "$CAS"
# capsule 7f3a… restored (2 files, 38 bytes) to /tmp/world-B

diff -r /tmp/world-A /tmp/world-B   # no output → byte-identical
wanix qjs --cwd /tmp/world-B scripts/hello.js
# hello, capsule
```

### Different machine — ship the blobs

The simplest portable form today is "the blobs the capsule references" — copy
the manifest blob and each file blob it names from the source store to the
destination store. A quick port-by-rsync:

```sh
# On host A:
( echo "$CAPSULE_ID"; awk '{print $1}' "$CAS/$CAPSULE_ID" ) \
  | xargs -I{} echo "$CAS/{}" \
  > /tmp/capsule-files.txt

rsync -av --files-from=/tmp/capsule-files.txt / hostB:/tmp/cas-B/

# On host B:
wanix capsule load "$CAPSULE_ID" /tmp/world-B --store /tmp/cas-B
```

### Over the mesh (no manual copy)

The async, network-fetching `IrohCasStore` is the same [`ContentStore`][store]
trait (see the crate-level doc in [`wanix-cas/src/lib.rs`][lib]). When the mesh
plane is wired up, the same `capsule load <id>` resolves blobs through the
iroh-blobs ticket the peer published — the local trust boundary
(`verify_hash`, size caps, `NormalizedPath`) still applies on materialize, so a
hostile remote cannot escape the target directory.

## 5. What is portable, what is not

Portable (the capsule guarantees it):

- File content under the world directory. Symlinks are intentionally skipped
  ([`collect_files`][capsule]) because a capsule freezes content, not link
  topology — a dereferenced symlink could escape the world root.
- Determinism. The sorted manifest means the same input tree freezes to the
  same id, every time, on every host. Equal files across capsules dedup to one
  blob.
- Integrity. Every blob is fetched whole and re-hashed against its content
  address before any byte is written to disk.

Not portable (do not expect a capsule to carry these):

- **Live mesh peers and tickets.** A capsule id is *itself* a content address;
  resolving it over the network needs the peer relationship and the iroh
  ticket, which are runtime state, not part of the capsule.
- **Ephemeral fds and handles.** `#task` exec sessions, open `#term/<id>`
  resources, in-flight 9P fids, `#cas/ingest` write handles — all gone the
  moment the originating process exits. A capsule that wants to "carry a
  running shell" needs the qjs snapshot/resume path
  (`wanix qjs-snapshot` / `qjs-resume`), which is a separate mechanism.
- **Live `#kv` state.** `#kv` is a service device, not a directory under the
  world root. Freeze KV state by copying the values you want into the world
  tree first (see section 1).
- **Symlinks, special files, extended attributes, permissions.** The on-disk
  form is plain files, mode `0o644` after materialize. If you need those,
  layer your own metadata file into the world.
- **Process identity, environment, cwd.** The capsule is a tree, not a task —
  re-launch policy is the caller's, not the capsule's.

## 6. Try the round-trip yourself

A minimal smoke test you can paste into a shell:

```sh
export CAS=$(mktemp -d)
SRC=$(mktemp -d); DST=$(mktemp -d)

echo "hello, capsule" > "$SRC/greeting.txt"
mkdir "$SRC/scripts"
cat > "$SRC/scripts/hello.js" <<'JS'
import * as std from "qjs:std";
std.out.puts(std.loadFile("greeting.txt"));
JS

ID=$(wanix capsule save "$SRC" --store "$CAS" | awk '/^capsule/ {print $2}')
wanix capsule load "$ID" "$DST" --store "$CAS"

diff -r "$SRC" "$DST" && echo "round-trip ok: $ID"
```

That id is the only durable thing you need to reconstruct that world.
