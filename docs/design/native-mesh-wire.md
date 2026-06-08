# Native Mesh Wire — Design & Op-by-Op Implementation Plan

Status: design accepted, unimplemented. Authoritative decision record:
[ADR 0004](../adrs/0004-rust-9p-protocol-and-server-contract.md). Per-principal
trust boundary: [ADR 0006](../adrs/0006-serve-9p-transport-and-trust-boundary.md).
Forcing function for the per-principal identity this wire bakes in:
[ADR 0007](../adrs/0007-resources-catalogs-and-pairing.md).

This document specifies the **native FileSystem-over-iroh wire** that replaces
9P-over-QUIC for Wanix↔Wanix mesh traffic, and gives an implementer an
op-by-op, file-by-file, phased plan to build it. 9P is **not** removed: it stays
exactly where it is at the foreign edge (Linux `v9fs`, v86/QEMU virtio-9p,
external 9P tools, the cockpit `workbench/src/wanix/p9.ts`). Nothing in
`wanix-9p`, `wanix-9p-client`, or `wanix-protocol` changes; the only swap is at
the mesh import sites.

---

## 1. Chosen approach: hand-rolled frame, not `irpc` — and why

ADR 0004 §Decision names two options for the native wire: **`irpc` as primary**,
with **"a hand-rolled minimal frame over raw QUIC streams" as the explicit
fallback "if the open-file/streaming mapping proves awkward."** We take the
fallback, and we take it deliberately, judged through Rich Hickey's
**simple-not-easy** lens (the project's design mandate): when *simple* (not
complected, few concepts, total control, low lock-in) and *easy* (low effort to
start, familiar, close-at-hand) conflict, prefer simple.

Both options were compile-spiked against the locked versions
(`iroh =1.0.0-rc.1`, `irpc =0.16.0`, `postcard =1.1.3`, `n0-error =1.0.0-rc.0`,
`noq =1.0.0-rc.1` — all present in `Cargo.lock`); both built and round-tripped,
so neither is disqualified on the build criterion. The tie-break is the
simple-not-easy weighting. Spikes were thrown away (detached `/tmp` cargo
projects); the repo was never touched.

**What `irpc` is (the easy option), and why it loses on its own terms.** `irpc`
being already transitively in `Cargo.lock` is an *easy* fact ("no new top-level
dep, familiar RPC idiom"), not a *simple* one. Simplicity is about complecting,
and there `irpc` braids the wire encoding (the part we need) together with an
actor/`Client`/`Service`/`RemoteConnection`/`LocalSender`/`WithChannels` concept
stack, `n0-error`, a forced `tracing` dependency (the `rpc_requests` derive
expands to `::tracing` under the default `spans` feature), and a 16 MiB
`MAX_MESSAGE_SIZE` policy. Decisively, both the transport spike and the identity
research independently found that `irpc`'s turnkey server loop
(`rpc::handle_connection` / `listen` / `read_request`) is **unusable for the two
non-negotiable requirements**:

1. **Per-principal identity** — those helpers take a bare `noq::Connection`,
   expose only a socket address, and **discard** the ed25519 `remote_id()`. The
   verified pubkey lives only on the *iroh* `Connection` wrapper
   (`conn.remote_id() -> EndpointId`), which does not hand out its private
   `inner: noq::Connection`. So `read_request::<S>(&iroh_connection)` does not
   even type-check.
2. Consequently you **must hand-roll the accept loop** anyway, read the length
   prefix yourself, and call `with_remote_channels` directly — at which point
   `irpc`'s headline value (turnkey connection handling) is exactly the part you
   throw away, while its machinery stays, and the hand-rolled reader is now
   coupled to `irpc`'s *internal* frame format (varint prefix + an optional
   `SpanContextCarrier` tuple) that can drift silently across an `irpc` bump.

**What hand-rolled decouples.** One concept stack: `postcard`
`Serialize`/`Deserialize` + a length-prefixed frame +
`iroh::Connection::open_bi`/`accept_bi`. The QUIC stream identity **is** the
transaction (no tags — ADR 0004's point); per-stream flow control replaces
`msize`; stream-drop replaces `Tclunk`. The precedent is **already shipping
twice in the tree**: `crates/wanix-cpu/src/wire.rs` and `event.rs` are typed
structs, length-prefixed framing over blocking `Read`/`Write`, bounded decode
against explicit `MAX_FIELD_LEN`/`MAX_LIST_LEN` ceilings. The native wire is
literally *"`wanix-cpu`'s wire generalized to the FileSystem op-set, `postcard`
instead of hand-written field codecs."*

**The single biggest structural win — the sync/async bridge is already solved.**
`crates/wanix-9p-client/src/transport.rs` already defines the transport boundary
as the **sync `Duplex: Read + Write + Send`** trait with its own bounded-frame
ceiling (`read_one_frame`, `transport.rs:47`). `crates/wanix-mesh/src/duplex.rs`
already bridges async iroh streams to it (`BlockingDuplex`/`BlockingReader`/
`BlockingWriter`) via `block_on` on a held `tokio::runtime::Handle`, with the
documented hazard (panics on a runtime worker thread; used only on
OS/`spawn_blocking` threads). **The hand-rolled wire reuses this already-shipped
bridge verbatim, with zero new bridge code.** `irpc` would instead layer its
async `Client`/`mpsc` runtime *on top of* the sync `FileSystem` boundary and
re-introduce a *second* sync-over-async seam inside it. That is structural, not
stylistic, and it is the deciding simplicity win.

**What both share (not differentiators).** Both eliminate the lossy
`FsError → errno → FsError` table (the spike round-tripped
`InvalidPath("a/../b")` with the `String` intact; cf. the current lossy
`EINVAL → FsError::Other("remote errno 22")` at
`crates/wanix-9p-client/src/error.rs:54`). Both fold `StreamingImportFs` and its
`StreamPredicate` out of existence because every open file rides its own QUIC
stream by construction. Both bind the principal once per connection from
`conn.remote_id()` and reuse `wanix-id`'s `AttachPolicy`/`GrantTable` unchanged.
Since both deliver the typed-error, per-stream, and per-principal wins equally,
the choice is decided purely on complecting / concept count / control /
lock-in — where hand-rolled wins on every axis.

**The honest cost (the "not easy" tax).** Hand-rolled is *more effort*:
~100–150 LOC of framing/dispatch boilerplate that `irpc`'s derive would generate,
and we own backpressure / EOF / drop semantics by hand on the open-file stream.
That is the correct trade: fewer interleaved concerns, total wire control,
near-zero added lock-in (no `irpc`/`n0-error`/macro surface to track across the
pre-release `noq`/`n0` churn), full debuggability (every byte hexdumpable; a hang
is "this one stream," not generated channel/runtime code). **Simple over easy,
exactly as mandated, and within the envelope ADR 0004 explicitly sanctioned.**

---

## 2. The new crate and its place in the dependency graph

Create **`crates/wanix-mesh-wire`**: the typed wire *contract and codec*, and the
sync client `FileSystem` facade + the sync server dispatcher. It is the native
analog of `wanix-9p` (server codec) + `wanix-9p-client` (client codec), and like
them it is **transport-agnostic and async-free** — it speaks over the existing
sync `Duplex` boundary. `wanix-mesh` then *composes* this codec onto its iroh
endpoint exactly as it composes the 9P codec today.

```text
wanix-mesh-wire -> wanix-fs + wanix-vfs + serde + postcard
                   (NO iroh, NO tokio, NO irpc, NO noq)

wanix-mesh -> wanix-mesh-wire + wanix-9p + wanix-9p-client + ... + iroh/tokio
              (the ONE async/iroh edge; supplies QUIC streams + the held Handle)
```

This split is load-bearing for the guardrail *"`wanix-mesh` is the single
async/iroh edge; keep tokio and iroh out of every other crate."* The wire crate
defines the codec over a generic byte stream (`Duplex`), and `wanix-mesh` wraps
each iroh QUIC bidi stream in the existing `BlockingDuplex` and drives it on the
held `Handle`. The wire crate never sees async; `wanix-mesh` never invents a new
bridge.

`serde` (1.0.228) and `postcard` (1.1.3) are already resolved in `Cargo.lock`
(transitive). Add them as **direct, workspace** dependencies of
`wanix-mesh-wire` (and to the workspace `Cargo.toml` `members`). The wire crate
should stay under the 250-line/module warn limit per the code-quality guardrails;
split into `frame.rs` / `value.rs` (mirror types) / `proto.rs` (request/response
enums) / `client.rs` (`NativeFs`/`NativeFile`) / `server.rs` (dispatch) from the
start.

---

## 3. Wire model: frames, streams, and the service shape

### 3.1 The two stream kinds

The wire uses iroh bidi QUIC streams in exactly two shapes. There are **no tags**
(a stream is the transaction) and **no `msize`** (QUIC flow control bounds
memory; we keep an advisory `iounit`-style chunk hint).

- **One-shot op stream.** A non-`open` `FileSystem` method (`metadata`,
  `read_dir`, `create_dir`, `remove_*`, `rename`, `symlink`, `hard_link`,
  `read_link`, `set_permissions`, `set_times`, `content_hash`) opens a fresh bidi
  stream, writes exactly **one** `FsRequest` frame, reads exactly **one**
  `FsResponse` frame, and drops the stream. Request/response, then teardown.
  This mirrors `RemoteFs`'s `Arc<Mutex<P9Conn>>` discipline but without the
  shared serial connection: each op is independent, so there is no per-call mutex
  contention and no head-of-line coupling between ops.

- **Open-file stream (stateful, fid-shaped).** `open()` opens a bidi stream
  dedicated to that one `File` for its whole lifetime. The client writes one
  `OpenRequest` frame; the server replies with one `OpenResponse` carrying
  `{seekable, iounit_hint, initial Metadata}` (so the handle knows its lifecycle
  up front, no second round trip). Thereafter the stream carries a duplex
  sub-protocol:
  - **client → server**: `FileOp` frames (`Read{max}`, `Write(bytes)`,
    `Seek{whence, offset}`, `SetLen{len}`, `Stat`, `Close`);
  - **server → client**: `FileReply` frames (`Chunk(bytes)`, `Wrote(usize)`,
    `Seeked(u64)`, `Ok`, `Stat(WireMetadata)`, `Err(WireFsError)`).
  Dropping the `File` finishes the client send half; the server observes the
  half-close and tears down its reader. This **replaces `Tclunk`**. Because every
  open file is on its own stream, a never-EOF read (`#agent/<id>/events`,
  `#agent/<id>/reply`, `#plumb/<topic>/recv`) parks **only its own stream** and
  cannot freeze any sibling op — which is precisely what `StreamingImportFs`
  hand-discovers today (`crates/wanix-mesh/src/streaming.rs`), made structural.

### 3.2 Frame discipline (bounded decode)

Every frame is **`4-byte little-endian length prefix` + `postcard body`**,
exactly the ceiling-discipline of `transport.rs:47` and `wanix-cpu/src/wire.rs`.
Reuse it: read the 4-byte prefix, reject any body over an explicit ceiling
**before allocating**, then read exactly that many bytes and `postcard`-decode.
This protects an importer/exporter from a hostile peer forcing an unbounded
allocation. Ceilings (named constants in `frame.rs`):

- `MAX_FRAME_LEN` (e.g. 1 MiB) — control frames (requests, responses, metadata,
  a `readdir` page).
- `MAX_CHUNK_LEN` (e.g. 256 KiB) — a single `Chunk`/`Write` payload. This is the
  **server-side read-chunk cap**, independent of the client's requested
  `Read{max}`: a client looping `Read` with a huge `max` must not force a large
  server buffer; the server clamps each `Chunk` to `min(client_max,
  MAX_CHUNK_LEN, iounit_hint)`. The `iounit_hint` is advisory only.

### 3.3 The op-level service enum

`proto.rs` defines (all `#[derive(Serialize, Deserialize)]`):

```rust
enum FsRequest {
    Stat       { path: String, follow_symlink: bool },     // metadata / metadata_with_lookup
    ReadDir    { path: String },                            // bulk; server may page (see §4)
    ReadLink   { path: String },
    Symlink    { target: Vec<u8>, path: String },
    HardLink   { old_path: String, new_path: String },
    CreateDir  { path: String },
    Remove     { path: String, dir: bool },                // remove_file / remove_dir
    Rename     { old_path: String, new_path: String },
    SetPermissions { path: String, permissions: u32 },
    SetTimes   { path: String, accessed_time_ns: u64, modified_time_ns: u64 },
    ContentHash{ path: String },
    // Open is its own stream-opening request (OpenRequest), not a one-shot.
}

enum FsResponse {
    Unit(Result<(), WireFsError>),
    Stat(Result<WireMetadata, WireFsError>),
    ReadDir(Result<ReadDirPage, WireFsError>),            // see §4 for paging
    ReadLink(Result<Vec<u8>, WireFsError>),
    ContentHash(Result<Option<[u8; 32]>, WireFsError>),
}

struct OpenRequest { path: String, options: WireOpenOptions, append: bool }
struct WireOpenOptions { read: bool, write: bool, create: bool, truncate: bool }
struct OpenResponse(Result<OpenOk, WireFsError>);
struct OpenOk { seekable: bool, iounit_hint: u32, metadata: WireMetadata }

enum FileOp   { Read { max: u32 }, Write(Vec<u8>), Seek(WireSeek), SetLen(u64), Stat, Close }
enum WireSeek { Start(u64), Current(i64), End(i64) }
enum FileReply {
    Chunk(Vec<u8>),                 // 0-length Chunk == EOF for regular files
    Eof,                            // explicit device close for never-EOF streams
    Wrote(u32),
    Seeked(u64),
    Ok,
    Stat(WireMetadata),
    Err(WireFsError),
}
```

The first frame on a stream selects the kind: the dialer writes either an
`FsRequest`-tagged one-shot frame or an `OpenRequest`-tagged frame. (Use one
top-level `enum Inbound { OneShot(FsRequest), Open(OpenRequest) }` so the server
reads one frame and dispatches.) Writing that first frame is also what makes the
peer's `accept_bi` resolve — an iroh bidi stream is invisible to the acceptor
until the opener writes its first byte (the same fact `dialer.rs`/`duplex.rs`
rely on today).

**`confine_to_prefix` is NOT on the wire.** It is the local-only `SubtreeFs`
re-rooting confinement hook (`traits.rs:229`), enforced server-side on the
exported `dyn FileSystem` before following symlinks. It never crosses the wire.

---

## 4. Op-by-op mapping (FileSystem + File → wire)

The wire surface is the `wanix_fs::FileSystem` + `wanix_fs::File` traits **only**
(`crates/wanix-fs/src/traits.rs`). `NamespaceOps` is not a separate wire concern:
`wanix_vfs::Namespace` *implements* `FileSystem`, so the mesh exports/imports a
`dyn FileSystem` and the namespace/bind layer never crosses the wire. The native
wire carries **all** trait methods (including the ones `RemoteFs` does not bridge
today — `symlink`, `hard_link`, `set_permissions`, `set_times`,
`metadata_with_lookup`), so it is a *faithful* `FileSystem` encoding, not just
the 9P-bridged subset.

| Trait method | Wire op | Notes |
|---|---|---|
| `open(path, opts)` → `Box<dyn File>` | `OpenRequest` opens a dedicated stream; `OpenResponse` returns `{seekable, iounit_hint, metadata}` | The single stateful op. `append` is the extra flag the 9P client carries (`open_impl(path, opts, append)`); `NativeFs::open` passes `append=false`, matching `RemoteFs::open` today (`remote.rs:230`). A future append-aware path sets the flag. |
| `metadata(path)` | `FsRequest::Stat{path, follow_symlink:true}` | Compound path-resolve+stat in **one** round trip, replacing 9P's `walk → getattr` (or its `Twalkgetattr` collapse). |
| `metadata_with_lookup(path, lookup)` | `FsRequest::Stat{path, follow_symlink: lookup.follow_symlinks()}` | Same op; `NoFollow` sets `follow_symlink=false`. |
| `read_dir(path)` | `FsRequest::ReadDir{path}` → `ReadDirPage` | **Bulk** (one reply), with optional server-streaming pages for huge dirs (see below). Each `WireDirEntry` carries **full** `WireMetadata` for free (no 9P placeholder-size problem). |
| `read_link(path)` | `FsRequest::ReadLink{path}` → `Vec<u8>` | Uninterpreted target bytes. |
| `symlink(target, path)` | `FsRequest::Symlink{target, path}` | **New on the wire** (not 9P-bridged today). |
| `hard_link(old, new)` | `FsRequest::HardLink{old_path, new_path}` | New on the wire. |
| `create_dir(path)` | `FsRequest::CreateDir{path}` | |
| `remove_file(path)` | `FsRequest::Remove{path, dir:false}` | |
| `remove_dir(path)` | `FsRequest::Remove{path, dir:true}` | |
| `rename(old, new)` | `FsRequest::Rename{old_path, new_path}` | |
| `set_permissions(path, perms)` | `FsRequest::SetPermissions{path, permissions}` | New on the wire. |
| `set_times(path, a_ns, m_ns)` | `FsRequest::SetTimes{path, accessed_time_ns, modified_time_ns}` | New on the wire. Two u64 nanosecond stamps. |
| `content_hash(path)` | `FsRequest::ContentHash{path}` → `Option<[u8;32]>` | **Typed** `Option<ContentHash>`, no `cas.hash` xattr/string hack (cf. `remote.rs:203`). The control/data-plane split hook stays: a CAS-aware caller still fetches the BLAKE3 blob off the iroh-blobs data plane instead of streaming bytes. |
| `confine_to_prefix` | — | Local-only; never on the wire. |

### File handle ops (on the open file's own stream)

| `File` method | Wire | Notes |
|---|---|---|
| `read(buf)` | client `FileOp::Read{max}` → server `FileReply::Chunk(bytes)` (or `Eof`) | Server clamps `Chunk` to `min(max, MAX_CHUNK_LEN, iounit_hint)`. A 0-length `Chunk` is EOF for regular files; never-EOF devices send `Chunk`s and only ever send `Eof` on real device close. |
| `write(buf)` | client `FileOp::Write(bytes)` → server `FileReply::Wrote(n)` | Default `NotSupported` server-side becomes `FileReply::Err(WireFsError::NotSupported)`. |
| `seek(from)` | **client-side for regular files** (track offset locally; `End(delta)` does a `FileOp::Stat` for len) **or** `FileOp::Seek` for server-authoritative seek | Keep the `RemoteFile` approach (`file.rs:140`): client-side offset for seekable regular files; non-regular files report `is_seekable()==false` and refuse `seek`. The `iounit_hint`/`seekable` come from `OpenOk` so no extra probe is needed. |
| `tell()` | client-tracked | As `RemoteFile::tell` (`file.rs:157`). |
| `is_seekable()` | from `OpenOk.seekable` | Set at open from the server's file-type probe, carried in the open response (replaces the `Tgetattr` round trip at `remote.rs:211`). |
| `set_len(len)` | `FileOp::SetLen(len)` → `FileReply::Ok`/`Err` | New (not bridged by `RemoteFile` today). |
| `read_ready()` / `write_ready()` | optional `FileOp::ReadReady`/`WriteReady` → bool, or default `Ok(true)` | Devices *could* surface real readiness here; default to `Ok(true)` to match `RemoteFile`. |
| `metadata()` | `FileOp::Stat` → `FileReply::Stat(WireMetadata)` | On the open handle, as `RemoteFile::metadata` (`file.rs:169`). |
| `Drop` | finish the client send half | Server observes the half-close and releases the reader; replaces `Tclunk`. |

### Bulk readdir paging

`read_dir` returns `Vec<DirEntry>` and the 9P client pages it under
`MAX_ENTRIES = 1_000_000` / `MAX_ITERATIONS = 100_000` (`readdir.rs`). On the
native wire keep it **bulk by default** (one `ReadDirPage` reply) but make
`ReadDirPage` a `{ entries: Vec<WireDirEntry>, more: bool }` and, when `more`, let
the client send a follow-up `FsRequest::ReadDir` continuation on the *same*
stream (a `cookie: Option<u64>` field), so a multi-million-entry directory
streams in `MAX_FRAME_LEN`-bounded pages without a single unbounded frame. Carry
the same total/iteration ceilings client-side.

---

## 5. Typed `FsError` on the wire (`WireFsError`)

The 9P path round-trips `FsError → Linux errno → FsError` **lossily and
non-injectively**: `InvalidPath`/`InvalidOffset`/`InvalidTime`/`Other` all
collapse onto `EINVAL`/`Other`, so `InvalidPath(s)` and its message are **lost**
(confirmed at `crates/wanix-9p-client/src/error.rs:54` →
`FsError::Other("remote errno 22")`). The native wire **puts a typed mirror on
the wire**: every op returns `Result<T, WireFsError>` encoded as `postcard`.

`WireFsError` is a `#[derive(Serialize, Deserialize)]` mirror of
`wanix_fs::FsError` (`crates/wanix-fs/src/error.rs`), with a `From<FsError>` /
`Into<FsError>` pair, defined **in `wanix-mesh-wire`**:

```rust
enum WireFsError {
    InvalidPath(String), NotFound, NotSupported, PermissionDenied,
    AlreadyExists, NotDirectory, IsDirectory, InvalidFd, InvalidOffset,
    InvalidTime, NotEmpty, Other(String),
}
```

**Do not** add `serde` derives to `wanix_fs::FsError`/`Metadata`/`DirEntry`. Those
are leaf core types with private fields and constructors and carry no serde
today; mirroring at the wire edge keeps `wanix-fs` dependency-free and matches
how `wanix-9p/src/attr.rs` already maps `P9Attr ↔ Metadata`. The round trip is
**lossless**: `InvalidPath("a/../b")` arrives with its `String` intact.

The transport-failure surface (dead connection, stream reset, decode failure)
stays **separate** from `WireFsError` and lowers to `FsError::Other` only on a
genuine transport fault — analogous to `ClientError::Io`/`Poisoned → Other` today
(`error.rs:92`). A `WireFsError` is an *application* error the server chose to
return; an `FsError::Other("mesh: ...")` is a *transport* fault.

### Mirror value types (round-trip identity is mandatory)

`value.rs` defines `WireMetadata`, `WireDirEntry`, `WireFileType`,
`WireOpenOptions`, `WireSeek`, with `From`/`Into` converters. `Metadata`/
`DirEntry` have **private fields** rebuilt via `Metadata::new_with_links` /
`MetadataTimes::new` / `DirEntry::new` (`crates/wanix-fs/src/metadata.rs`), so a
field-order or unit (ns vs s) mismatch would **silently corrupt** metadata with
no compile error. This is a named risk; mitigate with round-trip identity tests:

```text
Metadata -> WireMetadata -> Metadata == identity   (all 7 fields)
DirEntry -> WireDirEntry -> DirEntry == identity
```

`WireMetadata` carries exactly: `file_type ∈ {File,Directory,Symlink}`, `len:u64`,
`mode:u32`, `link_count:u64`, `accessed_time_ns:u64`, `modified_time_ns:u64`,
`changed_time_ns:u64`.

---

## 6. Per-principal identity threading

This wire is the *first consumer* of the per-principal seam ADR 0006/0007 need,
and it gets it for almost free because the verified principal is **already in
hand at exactly the right place**. The chosen shape is **a per-connection
principal-scoped `FileSystem` view, NOT a principal argument on every op.**

### Why per-connection view, not per-op argument

A per-op principal parameter would complect authorization with every method
signature, force the principal onto the wire (it must come from the *transport*,
not the payload — ADR 0007 §chatroom: "identity comes from the transport, not the
payload"), and touch the core `FileSystem`/`File` traits that ADR 0004 mandates
stay **untouched**. The core is entirely principal-blind today (zero `PeerId`
references in `wanix-fs`/`wanix-vfs`/`wanix-kv`/`wanix-plumb`/`wanix-agent`), and
it must stay so.

Instead: at connection-accept, resolve the principal → a principal-specific
`Arc<dyn FileSystem>` view via the existing `AttachPolicy`, then serve every op
on that connection against that one view. The principal is bound to the
**server-side handler**, not carried on the wire. This is **literally the model
already shipping for 9P**: `P9Server::with_policy(default_root, peer, policy)`
(`crates/wanix-9p/src/lib.rs:137`) stores `AttachContext{peer, policy}` and
installs the policy-scoped `SubtreeFs` as the connection root. The native server
reuses this unchanged.

### The three-hop resolution (identical to 9P, minus the `Tattach`/`uname`
ceremony)

```text
connection.remote_id()                         // iroh EndpointId, TLS-proven, pre-app
  -> wanix_mesh::identity::peer_id_for(..)      // -> wanix_id::PeerId  (identity.rs:29)
  -> AttachPolicy::evaluate(peer, aname)        // -> Option<Authorization>  (policy.rs:14)
  -> Authorization{ root: Arc<dyn FileSystem>,  // a SubtreeFs that ALSO enforces Rights
                    rights: Rights }            // grant.rs:16,96
  -> that root is THE per-connection view
```

`None` from `evaluate` is **default-deny**: refuse the connection (the native
analog of the 9P `EACCES` attach rejection). When no policy is configured
(loopback / fully trusted), serve the wholesale root, exactly as `ServeConfig`
does today (`handler.rs:42`/`:52`).

This **resolves the queued `NamespaceProvider`/`uname` seam correctly**. The
CLAUDE.md note warns against a no-op `NamespaceProvider` trait whose result is
discarded (9P `handle_attach` decodes `uname`/`aname` and throws them away). This
design is the **non-no-op** version: the provider is `AttachPolicy` (already
exists, already consumed), its result — the per-connection root — is *actually
used* for the whole connection, and the principal is the cryptographic `PeerId`
from `remote_id()`, never a client-claimed `uname`. There is no dead abstraction
because there is no new abstraction.

### What ADR 0007 builds on top

A `#chat`/resource device served over this wire is just a `FileSystem`; its
per-connection view already knows the principal, so attribution (`post` stamped
with the authenticated pubkey, ignoring any client-claimed author), `who`
(the set of live `remote_id()`s), and layer-2 ACLs (`GrantTable` `grant`/`revoke`
= invite/kick) are all *policy/ownership* problems over the raw `PeerId` — the
identity half is already done. Petnames live in the catalog/UI layer over the raw
`PeerId`; the wire only ever carries the pubkey, which is correct.

**Both planes must select the same root.** The native server and the 9P gateway
must resolve the *same* `AttachPolicy::evaluate(peer, aname)` → `Authorization`
for a given peer, so neither plane grants wider access than the other.

---

## 7. Server export change in `wanix-mesh`

Add a **second ALPN** beside `WANIX_9P_ALPN`. Define
`pub const WANIX_FS_ALPN: &[u8] = b"wanix/fs/1";` in `wanix-mesh/src/lib.rs`. The
native handler rides the same identity-bound endpoint and `Router` as 9P; the
multi-ALPN pattern is already proven three times (`serve_with_blobs`,
`serve_with_plumb`, `serve_cpu` in `node.rs`).

New module `wanix-mesh/src/wire_handler.rs` (the structural analog of
`handler.rs`):

```rust
impl ProtocolHandler for NativeFsHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = peer_id_for(connection.remote_id());          // once per connection
        // Resolve the per-connection root via AttachPolicy (default-deny on None).
        // For v1 single-attach, evaluate the empty/aname root here, exactly as
        // P9Server::with_policy installs on first Tattach.
        loop {
            let (send, recv) = match connection.accept_bi().await { Ok(s)=>s, Err(_)=>return Ok(()) };
            let Ok(permit) = Arc::clone(&self.sessions).acquire_owned().await else { return Ok(()) };
            let root = self.config.resolve_root(peer);            // Arc<dyn FileSystem>
            let handle = self.handle.clone();
            let deadline = self.config.deadline;
            tokio::task::spawn_blocking(move || {
                // Wrap the QUIC bidi stream in the EXISTING BlockingDuplex, then
                // run the SYNC NativeServer dispatch over it. One stream == one op
                // (one-shot) or one open file (streaming).
                let duplex = BlockingDuplex::new(send, recv, handle, /* deadline */ None);
                wanix_mesh_wire::serve_one(&root, duplex, deadline);   // sync
                drop(permit);
            });
        }
    }
}
```

Key points, each a named risk to honor:

- **Dispatch into the sync `FileSystem` runs on `spawn_blocking`** (the blocking
  pool), exactly as `serve_one_stream` does today (`handler.rs:128`). A streaming
  read on a never-EOF device **blocks** the calling thread; it must block a
  blocking-pool thread, never a runtime worker. Mirror `node.rs:96-109`'s explicit
  blocking-pool sizing (`MAX_CONCURRENT_SESSIONS = 512`, one pinned thread per
  live open file).
- **Streaming reads must NOT inherit the per-op deadline.** `handler.rs:151-159`
  already documents this: a `DEFAULT_OP_DEADLINE` on the *idle read* would tear
  down a live, mounted-but-idle session. Carry the same nuance: the deadline
  bounds in-flight *writes* (a stalled peer must not park a thread forever) but
  **not** the server's blocking read on a never-EOF `#agent/events` subscription.
  In the open-file sub-protocol, the server's read of the next `FileOp` is the
  idle wait → no deadline; the server's send of a `Chunk`/`FileReply` → deadline.
- **Server-side `Chunk` cap independent of client `Read{max}`** (§3.2): clamp to
  `min(max, MAX_CHUNK_LEN, iounit_hint)`.
- **Per-connection root resolution reuses `AttachPolicy`/`GrantTable`/`SubtreeFs`
  unchanged** (§6). The native `ServeConfig` mirrors the 9P `ServeConfig`
  (`handler.rs:33`): `{ root, policy: Option<Arc<dyn AttachPolicy>>, deadline }`.

`MeshNode` gains `serve_native(config)` (and `serve_native_with_blobs`, etc., as
the multi-plane needs arise) building a `Router` with `.accept(WANIX_FS_ALPN,
native_handler)` — possibly alongside `WANIX_9P_ALPN` during the transition so a
node speaks both wires.

---

## 8. Client native `Fs` + the sync/async bridge

`wanix-mesh-wire::NativeFs` is a sync `wanix_fs::FileSystem` that mirrors
`RemoteFs`'s shape. It holds the means to open a fresh bidi stream per op (in
`wanix-mesh`'s terms, the iroh `Connection` + held `Handle`), and for each op:

1. open a bidi stream (one-shot) or the dedicated open-file stream;
2. `write_frame` one `postcard` request;
3. `read_frame` one `postcard` response (bounded decode);
4. map `WireFsError → FsError`, return.

`NativeFile` owns its own bidi stream and runs the `FileOp`/`FileReply`
sub-protocol on it; `Drop` finishes the send half.

**The bridge is the existing one, unchanged.** The wire crate's `NativeFs` is
defined over a *stream factory* abstraction it can call synchronously; in
`wanix-mesh`, that factory opens an iroh bidi stream via the held `Handle`
(`handle.block_on(conn.open_bi())`) and wraps it in the **existing
`BlockingDuplex`**. `NativeFs`/`NativeFile` then do sync `Read`/`Write` framing on
top — identical to how `RemoteFs` is fed today (`dialer.rs:114`,
`duplex.rs:131`). **No new bridge concept is introduced**; the documented
runtime-worker-thread hazard discipline (`duplex.rs:17-26`) carries over verbatim,
and so does the runtime-ownership trap: the owning `IrohMount`/node must outlive
every op because `BlockingDuplex` holds a *non-owning* `Handle`
(`ticket.rs:106`); drop the node and the next op panics with "Tokio context is
shutting down."

Concretely, the wire crate exposes a small sync seam the mesh implements:

```rust
// in wanix-mesh-wire
pub trait StreamFactory: Send + Sync {
    fn open_stream(&self) -> std::io::Result<Box<dyn Duplex>>;   // one fresh bidi stream
}
pub struct NativeFs<F: StreamFactory> { factory: F, /* ... */ }
// in wanix-mesh: impl StreamFactory by handle.block_on(connection.open_bi()) + BlockingDuplex
```

This keeps `wanix-mesh-wire` iroh/tokio-free (it only sees `Duplex`), and keeps
the entire async/iroh binding in `wanix-mesh`.

**Connection liveness / teardown.** Each non-streaming op opens and tears down a
fresh stream, so a dead connection surfaces per-op as an `open_bi`/read error →
`FsError::Other` (a genuine transport fault). Define a clear reconnection policy
(the simplest: the import is invalidated and re-dialed by the caller, as today),
and preserve the runtime-ownership discipline above.

---

## 9. The import-site swap (RemoteFs → NativeFs), 9P stays at the foreign edge

The swap is type-transparent because `bind` takes `impl FileSystem` and both
`RemoteFs` and `NativeFs` are `FileSystem`. The construction sites:

- **`crates/wanix-mesh/src/dialer.rs`** — `MeshDialer::dial`/`dial_attach`
  (`:61`/`:77`) build `RemoteFs::connect_with_aname` over a `BlockingDuplex`.
  Add native variants (or swap behind a flag during transition) that build a
  `NativeFs` over the iroh `Connection` (connecting on `WANIX_FS_ALPN`).
- **`crates/wanix-mesh/src/streaming.rs`** — `dial_streaming` →
  `StreamingImportFs`. **Retire this whole module** for the native wire: every
  open file is on its own stream by construction, so the dedicated-stream
  predicate hack is unnecessary. (Keep it only as long as the 9P import path
  exists for any mesh peer.)
- **`crates/wanix-cli/src/mesh/ticket.rs:129`** — `dial_iroh_remote` returns
  `IrohMount { remote: Arc<RemoteFs>, .. }`. Change `remote` to the native `Fs`
  type; keep the `IrohMount` keepalive discipline (it owns the runtime;
  `:106`).
- **`crates/wanix-cli/src/mount.rs:154`** — `namespace.bind(remote, ".",
  MOUNT_POINT, ..)` binds the imported FS at `/n/<peer>`. Type-transparent swap.
- **`crates/wanix-cli/src/mount.rs:197`** — `RemoteFs::connect` over a raw
  `TcpStream` (`tcp://`). **Leave on 9P** — this is the *foreign-edge* path, not
  mesh, and stays 9P per ADR 0004.
- **Tests** — `crates/wanix-mesh/tests/mesh_quic.rs` and the device tests
  (`mesh_agent.rs`, `mesh_plumb.rs`, `mesh_blobs.rs`) gain native-wire variants.

`wanix-9p`, `wanix-9p-client`, `wanix-protocol`, and the cockpit `p9.ts`
**do not change.** 9P remains the contract at the Linux/v86/QEMU/external/cockpit
edge.

**Cockpit caveat.** The browser cannot speak this wire (no raw QUIC in the
browser, no native-over-WS transport yet). The native wire does **not** replace
the cockpit's 9P/WS edge — intended per ADR 0004 (9P stays at the foreign +
browser edge) — which means the typed-error/per-principal benefits do not reach
the cockpit until a TS/wasm native client or a host-side gateway exists. Do not
let the native wire's existence imply the cockpit is covered.

---

## 10. Phased implementation plan (concrete file/crate list per phase)

Each phase ends green under `just check`. Phases 1–3 land the wire with **no**
async and **no** iroh, fully unit-testable over an in-memory `Duplex`
(`std::io::pipe` or a `Cursor` pair) — this is the largest, lowest-risk surface
and proves the codec before any QUIC is involved.

### Phase 1 — the wire crate skeleton + value/error mirrors (no transport)
Create `crates/wanix-mesh-wire`. Add to workspace `members`.
- `crates/wanix-mesh-wire/Cargo.toml` — deps: `wanix-fs`, `wanix-vfs`, `serde`
  (derive), `postcard`. **No iroh/tokio/irpc.**
- `src/lib.rs` — crate root, re-exports.
- `src/value.rs` — `WireMetadata`, `WireDirEntry`, `WireFileType`,
  `WireOpenOptions`, `WireSeek` + `From`/`Into` converters to the `wanix-fs`
  types via `Metadata::new_with_links`/`MetadataTimes::new`/`DirEntry::new`.
- `src/error.rs` — `WireFsError` + `From<FsError>`/`Into<FsError>`.
- `src/frame.rs` — `MAX_FRAME_LEN`, `MAX_CHUNK_LEN`, `write_frame`, `read_frame`
  (4-byte LE prefix + `postcard`, bounded decode) over `Read`/`Write`.
- **Tests (Phase 1)**: `Metadata→Wire→Metadata` and `DirEntry→Wire→DirEntry`
  identity (all fields); `FsError↔WireFsError` round trip incl.
  `InvalidPath(s)`/`Other(s)` `String` preservation; frame round-trip + an
  over-`MAX_FRAME_LEN` prefix rejected without allocating (mirror
  `transport.rs` tests); a `WireSeek`/`WireOpenOptions` round trip.

### Phase 2 — the protocol enums + sync server dispatch + sync client facade
Still no transport: server and client run over an in-memory `Duplex`.
- `src/proto.rs` — `Inbound{OneShot(FsRequest), Open(OpenRequest)}`, `FsRequest`,
  `FsResponse`, `ReadDirPage`, `OpenRequest`/`OpenResponse`/`OpenOk`, `FileOp`,
  `FileReply` (§3.3).
- `src/server.rs` — `serve_one(root: &Arc<dyn FileSystem>, duplex, deadline)`:
  read one `Inbound`; for `OneShot` dispatch to the trait method and write one
  `FsResponse`; for `Open` call `root.open(..)`, write `OpenResponse`, then run
  the open-file loop (`FileOp` in / `FileReply` out) until `Close`/half-close,
  clamping `Chunk` to `min(max, MAX_CHUNK_LEN, iounit_hint)`. **No deadline on the
  idle `FileOp` read.**
- `src/client.rs` — `StreamFactory` trait (§8), `NativeFs<F>: FileSystem`,
  `NativeFile: File`. Each op opens a stream via the factory, frames a request,
  reads a reply, maps the error. `NativeFile` owns its stream; `Drop` finishes
  the send half. Client-side seek/tell/offset tracking ported from
  `wanix-9p-client/src/file.rs`.
- **Tests (Phase 2 — the round-trip-a-FileSystem-over-the-wire proof)**: a
  `pipe`/thread harness wiring `NativeFs` ↔ `serve_one` over an in-process
  `Duplex`, run against a `MemFs`: open/read/write/seek; metadata follow vs
  nofollow; bulk readdir with full metadata; create_dir/remove/rename;
  symlink/read_link; set_permissions/set_times error or success per backing;
  content_hash `None`; a typed `WireFsError` (e.g. `InvalidPath`) surfaced as the
  *same* `FsError` variant (the typed-error proof); a never-EOF read on a fake
  blocking device delivering incremental chunks then a clean device close,
  proving streaming + non-EOF semantics with **no** dedicated-stream machinery.

### Phase 3 — module hygiene + conformance shape
- Keep each module ≤250 lines (split `server.rs`/`client.rs` if needed).
- `src/conformance.rs` (test-support) — a reusable `FileSystem`-contract
  conformance suite that can be run against **any** `dyn FileSystem`, used in
  Phase 5 against both a native import and a 9P import of the same `MemFs`.

### Phase 4 — bind the wire to QUIC in `wanix-mesh`
- `crates/wanix-mesh/src/lib.rs` — add `pub const WANIX_FS_ALPN: &[u8] =
  b"wanix/fs/1";` and re-exports.
- `crates/wanix-mesh/src/wire_handler.rs` — `NativeFsHandler:
  ProtocolHandler` (§7): per-connection `peer_id_for(remote_id())`, per-stream
  `spawn_blocking` running `wanix_mesh_wire::serve_one` over a `BlockingDuplex`;
  a native `ServeConfig` mirroring the 9P one (root + `Option<AttachPolicy>` +
  deadline) with `resolve_root(peer)`; the `Semaphore`/`MAX_CONCURRENT_SESSIONS`
  cap; **no deadline on the idle open-file read**.
- `crates/wanix-mesh/src/dialer.rs` — implement `wanix_mesh_wire::StreamFactory`
  over the iroh `Connection` + held `Handle` (`open_bi` + `BlockingDuplex`); add
  `dial_native`/`dial_native_attach` returning `NativeFs`.
- `crates/wanix-mesh/src/node.rs` — `serve_native(config)` (and multi-plane
  variants as needed) building a `Router` with `.accept(WANIX_FS_ALPN, ..)`;
  the endpoint `alpns(..)` list and `build_endpoint` updated to advertise
  `WANIX_FS_ALPN`.
- **Tests (Phase 4 — real QUIC round trip)**: a `wanix-mesh/tests/mesh_native.rs`
  modeled on `mesh_quic.rs`: two `bind_local` nodes, A `serve_native`, B
  `dial_native`, bind at `/n/A`, full round trip incl. a `#kv` write observed
  server-side (bytes-that-crossed-QUIC == bytes-stored); a never-EOF `#agent`/
  `#plumb` read on its own stream that does **not** stall a concurrent stat/read
  (head-of-line proof, with `StreamingImportFs` absent).

### Phase 5 — differential + per-principal proofs, then the import-site swap
- **Differential test** (`wanix-mesh/tests/mesh_native_differential.rs`): run the
  Phase 3 conformance suite against **both** a `NativeFs` import and a `RemoteFs`
  (9P) import of the *same* backing `MemFs`/services namespace, asserting
  behaviorally identical results (e.g. both honor/ignore read offset the same
  way) — the two-encodings-stay-equivalent guard, reusing the
  `shared_vfs_differential` pattern already in the tree.
- **Per-principal identity test** (`wanix-mesh/tests/mesh_native_identity.rs`):
  two distinct peer identities dialing one `serve_native` under a
  `GrantTablePolicy`; assert peer A (granted `projects/foo`) sees the scoped
  `SubtreeFs` root and peer B (no grant) is **denied** (default-deny), and that
  the native plane and a 9P plane over the same policy grant *identical* roots
  for the same peer.
- **Import-site swap** (§9): switch `dialer.rs`/`ticket.rs`/`mount.rs` mesh paths
  (NOT the `tcp://` foreign path) from `RemoteFs` to `NativeFs`; retire
  `streaming.rs`/`StreamPredicate` from the native path; update the device tests
  (`mesh_agent.rs`, `mesh_plumb.rs`, `mesh_blobs.rs`) to the native wire.
- **Cleanup pass**: confirm `wanix-mesh-wire` carries no iroh/tokio; confirm
  `just module-lines` green; update `rust-walkthrough.md` and the ADR 0004
  Status/Consequences (the chosen wire + this doc pointer).

---

## 11. Risk register (carry these into implementation)

1. **Async→sync dispatch seam** (the one part no spike fully wired): the accept
   loop is async, but each op calls the **sync** `dyn FileSystem` and a streaming
   read on a never-EOF device **blocks**. Dispatch MUST run on `spawn_blocking`
   (blocking pool), framing on a runtime worker, exactly as `handler.rs:128`.
   Size the blocking pool per `node.rs:96-109` (one pinned thread per live open
   file).
2. **Streaming reads must not inherit the per-op deadline** (`handler.rs:151`):
   no deadline on the idle `FileOp` read of a live subscription; deadline only on
   in-flight writes.
3. **Server `Chunk` cap independent of client `Read{max}`** (§3.2): clamp to
   `min(max, MAX_CHUNK_LEN, iounit_hint)`.
4. **`WireMetadata`/`WireDirEntry` converter correctness**: private fields,
   ns-vs-s units — a mismatch corrupts silently. Round-trip identity tests are
   mandatory (Phase 1).
5. **Two encodings must stay behaviorally equivalent** for shared backing
   filesystems (Phase 5 differential test); both planes must select the **same**
   `AttachPolicy` root for a peer.
6. **Connection liveness / runtime-ownership trap**: per-op `open_bi`/read errors
   → `FsError::Other`; the owning node/`IrohMount` must outlive every op
   (`BlockingDuplex` holds a non-owning `Handle`; `ticket.rs:106`).
7. **Bounded framing** (§3.2): reuse the `transport.rs:47`/`cpu/wire.rs` ceiling
   discipline so an untrusted peer cannot force an unbounded allocation; pin the
   exact framing with a round-trip test.
8. **Cockpit not covered** by the native wire (no browser QUIC) — by design, but
   state it so it is not assumed.
9. **ADR text**: ADR 0004 names `irpc` primary; record that the sanctioned
   fallback was taken and why (open-file/streaming simplicity + avoiding
   `irpc`/`n0` pre-release lock-in). Done in this cycle.

## 12. Fallback trigger (if hand-rolled streaming proves awkward)

Reconsider `irpc` **only** if hand-rolling correct backpressure + EOF +
mid-stream typed-error + clean drop/teardown on the open-file stream costs
materially more than the ~100–150 LOC `irpc`'s derive would save **and** cannot
be made robust with a focused test suite — exactly the "open-file/streaming
mapping proves awkward" case ADR 0004 names, inverted. A secondary trigger: a
near-term requirement for true client-streaming write ergonomics (large
flow-controlled multi-frame uploads) where `irpc`'s typed channel model is a
decisive simplification for *that op alone* — then reconsider `irpc` for that op,
not the base wire. Do **not** trigger on per-principal identity or the sync/async
bridge: both are equal across the two designs (`irpc` must hand-roll the accept
loop for the principal anyway, and the hand-rolled path reuses the existing
`Duplex`/`BlockingDuplex` bridge unchanged).
