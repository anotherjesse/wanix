# Wanix Mesh — Architecture Blueprint (synthesized, adversarially verified)

> **Update (native mesh wire implemented).** This blueprint was written for the
> first mesh build, which carried **9P over iroh QUIC** as the Wanix↔Wanix wire.
> That worked and shipped, but between two Wanix nodes — both of which speak the
> `FileSystem` trait natively — 9P is a middle layer that costs chattiness,
> single-stream head-of-line blocking, tags, and `msize`. The mesh now uses a
> **native FileSystem-over-iroh wire** (`wanix-mesh-wire`, ALPN `wanix/fs/1`) as
> the default Wanix↔Wanix path: one `postcard`-framed bidi QUIC stream per call
> and one per open file (no tags/`msize`), a **typed `WireFsError`** that fixes
> the very `errno_for_fs` non-injectivity this document flags in its first
> paragraph (`InvalidPath(s)` now survives the round trip), and per-connection
> principal identity bound from the verified `remote_id()`. Because every open
> file rides its own stream, `StreamingImportFs` is retired on the native path.
> **9P stays the foreign edge** (Linux/v86/QEMU, external 9P tools, the cockpit).
> The decision and op-by-op wire are in
> [docs/adrs/0004-rust-9p-protocol-and-server-contract.md](adrs/0004-rust-9p-protocol-and-server-contract.md)
> and [docs/design/native-mesh-wire.md](design/native-mesh-wire.md); the
> sections below remain valuable for the identity, capability, cpu, data-plane,
> and agent-mesh designs, which are unchanged, and for the 9P-import mechanics
> that the native wire generalizes off 9P.

All confirmed. The constants to invert (`errno_for_fs`, `O_*`, `attr_for_metadata`/`DT_*`/`P9_MODE_*`) are crate-private/`pub(super)`/`pub(crate)` in `wanix-9p` — so the "lift to `wanix-protocol`" correction is mandatory and real. `errno_for_fs` is non-injective (`EINVAL` ← InvalidPath/InvalidOffset/InvalidTime/Other), so the "exact inverse" claim is false; the client needs its own canonical errno→FsError table.

I now have enough ground truth to write a blueprint that is accurate to *this* tree rather than the phase0 tree the subsystems were verified against. Writing the document.

---

# Wanix Mesh — Architecture Blueprint

*Plan 9's cpu/import/export, factotum, venti, and plumber, made workable on the real internet — with agents as the operators humans never had.*

This is the build-from blueprint for turning Wanix into a mesh: every node a cryptographic identity, every node exporting its Plan 9 namespace, any node able to import another's and run tasks/agents against it. It synthesizes the verified 9p-client, transport, import/export, cpu, data-plane, trust, and agent-mesh subsystem designs into one consistent whole, with every adversarial correction baked in.

## 0. Ground truth (a historical snapshot of the tree this was written against)

> **What has changed since writing:** every claim in this section was true at
> writing time and is preserved as the baseline snapshot, but the workspace has
> moved: `wanix-kv`, `wanix-agent`, `wanix-pipe`, `wanix-cpu`, `wanix-cas`,
> `wanix-id`, `wanix-mesh`, `wanix-mesh-wire`, and the capsule path all exist
> now, and `wanix-mesh` pulls in iroh/tokio/quinn (confined to that one crate).
> See the slice status table in §4 for what shipped.

The subsystem designs were each verified against a *more advanced* tree (`wanix-qemu-phase0`) that already contains `#kv`, `#agent`, `ExecServer`, `AgentEngine`, `wanix_process_runner`, and a capsule path. **None of those exist on this `rust` branch.** I grepped the whole tree: zero hits for `#kv`, `#agent`, `#pipe`, `#cpu`, `capsule`, `AgentEngine`, `ExecServer`, `agent_program`. There is **no iroh, no tokio, no quinn** anywhere — not in `Cargo.lock`, not in any manifest. Every transport today is `std::net::TcpStream` + `std::thread::spawn`-per-connection, or `tungstenite` WS.

What *does* exist, verified line-by-line, is exactly the foundation the mesh needs:

- **`wanix-9p` `P9Server::serve_stream<R: Read, W: Write>`** (transport.rs:95) — a strictly serial loop: `reader.read()` → `frames.push()` → for each frame `handle_frame(&request)` (inline, `&mut self`) → `write_all(response)`. One in-flight request per stream. No pipelining. `handle_flush` (session.rs:53) is a **no-op stub** that returns `Rflush` and cancels nothing.
- **A complete bidirectional protocol codec** in `wanix-protocol`: every T-builder *and* every R-decoder the client needs already exists (`p9_tversion`/`p9_decode_rversion`, `p9_twalk`/`p9_decode_rwalk`, `p9_tread`/`p9_decode_rread`, … `p9_decode_rlerror`). The client is genuinely a state machine over existing codecs, not new wire code.
- **A 100%-synchronous `FileSystem`/`File` trait** (`FileSystem: Send + Sync`, traits.rs:153; `File: Send`, traits.rs:66). A sync client satisfies it with no async leakage.
- **`Namespace::bind(Arc<dyn FileSystem>, source, dest, BindOptions)`** (binding.rs:60) — takes `&mut self` and **eagerly calls `filesystem.metadata(&source)`** at bind time (binding.rs:69). Longest-prefix resolution. This is `/n/<node>` for free, but the eager metadata means binding a remote fs triggers a synchronous dial+attach+getattr inside `bind()`.
- `#term` and `#task` devices, qjs/wasm task drivers, `TaskTable::allocate_root_with_namespace` + `Task::bind` + `start`.

**Consequence for the blueprint:** we cannot "just bind `#kv` over the wire" because there is no `#kv` to bind. The mesh's first, irreducible deliverable is the **9P client filesystem**, demonstrated against the things that *do* exist (host root, `#term`, `#task`). `#kv`, `#agent`, `#cpu`, capsules, and the agent are built *on the mesh* as later slices, not assumed under it. This is the single biggest correction to the synthesized designs, and it makes the plan both honest and more achievable.

The one place the designs disagreed on a verifiable fact — whether iroh is "fantasy" (9p-client verdict) or "real APIs, wrong method names" (transport/cpu/data-plane verdicts) — resolves cleanly: **iroh is real software with a real, currently-churning API; it is simply absent from this repo.** Both are true. We treat iroh as a genuine but *unintroduced* dependency, confined to one edge crate, pinned to one wave, and validated signature-by-signature before we lean on it — and we ship the entire control plane over a local pipe and TCP *first*, so the mesh is provably correct before a single QUIC packet flies.

---

## 1. The big picture — one coherent mesh

```
        NODE A  (identity = ed25519 pubkey = EndpointId)            NODE B
   ┌───────────────────────────────────────────┐         ┌──────────────────────────┐
   │  Namespace (Plan9 bindings)                │         │  Namespace               │
   │   .        -> host root (LocalFs)          │         │   .     -> host root     │
   │   #term #task #kv #agent #cpu #plumb #cas   │   9P    │   /n/A  -> P9Client ──────┼──┐
   │                                            │ control │                          │  │
   │  P9Server::serve_stream (SERIAL, sync) ◄───┼─plane───┼─ P9Client (sync FS) ◄─────┘  │
   └──────────────┬─────────────────────────────┘  QUIC   └──────────────────────────┘  │
                  │ ALPN wanix/9p/1                bidi                                   │
                  │                                                                       │
   ┌──────────────┴───────────── one iroh Endpoint per node ──────────────────────────┐ │
   │  ALPN wanix/9p/1   -> 9P control plane  (walk/stat/small read/mutation)           │ │
   │  ALPN iroh-blobs   -> CAS data plane    (BLAKE3 bulk: worlds/capsules/modules) ◄──┼─┘
   │  ALPN iroh-gossip  -> #plumb bus        (typed pub/sub coordination)              │
   │  NodeID = ed25519 pubkey  = identity AND address; iroh does NAT/relay             │
   └───────────────────────────────────────────────────────────────────────────────────┘
                                  ▲
                          GrantTable (default-deny, keyed by peer EndpointId)
                          checked at accept(): chooses WHICH SubtreeFs each peer attaches
```

**Two planes, one identity.**

- **Control plane = 9P.** Walk, stat, small reads, directory listings, all mutations, and *service-file* I/O (`#kv`, `#task`, `#agent`, `#cpu`) ride 9P2000.L over a QUIC bidi stream. This is the namespace fabric: cheap to reason about, already implemented on the server side, and the exact thing the new client speaks. It is **chatty over WAN and we design around that**, not against it.
- **Bulk data plane = content-addressed blobs.** Large file bytes, frozen worlds, capsules, and compiled-module inputs move as BLAKE3-addressed iroh-blobs, peer-to-peer, verified end-to-end while streaming. The control plane *references* bulk by hash; the bytes never crawl through the 9P `msize` window. This is venti.
- **Coordination plane = gossip.** A `#plumb` device maps topic names to gossip topics for typed, best-effort pub/sub between agents and tools. This is the plumber. It is explicitly *not* a durable queue.

**Identity flows one direction only.** The QUIC handshake authenticates the peer's ed25519 key. The verified `EndpointId` is read from the accepted `Connection` (`Connection::remote_id()` — **not** `remote_node_id()`, which does not exist in current iroh; corrected from four subsystem designs) and injected into the per-stream `P9Server` *at construction*. The server never trusts a client-claimed `uname`; the cryptographic identity *is* the transport peer. Authorization is then a pure function `evaluate(peer, aname) -> Authorization { root: Arc<dyn FileSystem>, rights }`, and a grant *is literally a bind*: the root the peer attaches is a `SubtreeFs` re-rooting the host namespace at a granted subpath with a rights gate. `Tauth` stays `ENOSYS` forever — there is nothing to authenticate in-band, which is the whole point of "made cheap by iroh."

**The unifying insight:** every remote capability — a peer's files, its `#kv`, its `#task`, a running agent, a cpu job — is *just a `FileSystem` reachable through `Namespace` resolution at `/n/<node>/...`*. The mesh adds exactly one new core primitive (a `FileSystem` that speaks 9P to a remote server) and one new edge (iroh transport + identity). Everything else is composition of mechanisms Wanix already has.

---

## 2. Crate and module shape

Dependency direction is sacred: **core fs/vfs/protocol/9p/task stay free of iroh, tokio, and Wasmtime.** Async lives only at the edge. Here is the layering, with new crates marked `[NEW]`:

```
wanix-fs ───────────────────────────────────────────────┐  (ContentHash lives here; no deps)
  ├─ wanix-vfs        (Namespace; SubtreeFs lands HERE, not in a new id crate)
  ├─ wanix-protocol   (9P codec — GAINS lifted shared constants: see §2.1)
  │
  ├─ wanix-9p         (P9Server — GAINS optional AttachPolicy + peer; unchanged wire)
  │     └─ depends on wanix-fs + wanix-protocol
  │
  ├─ [NEW] wanix-9p-client    THE load-bearing primitive. P9Client + RemoteFs + RemoteFile.
  │            depends on wanix-fs + wanix-protocol ONLY. Sync. No transport, no iroh, no tokio.
  │            Generic over a `Duplex: Read + Write + Send` trait object.
  │
  ├─ wanix-task       (unchanged; later gains a #cpu acceptor reusing allocate_root+bind+start)
  │
  ├─ [NEW] wanix-id           NodeIdentity (ed25519), GrantTable, Grant, Authorization,
  │            AttachPolicy impl. depends on wanix-fs + wanix-vfs (for SubtreeFs) + ed25519-dalek.
  │            NO iroh: identity is a keypair, testable over a local pipe.
  │
  ├─ [NEW] wanix-cas          ContentHash newtype + ContentStore trait + LocalCasStore (BLAKE3,
  │            on-disk, reusing wanix-module-cache's audited owner-private/atomic-write boundary).
  │            depends on wanix-fs. NO iroh.  CasFs decorator + #cas device live here.
  │
  └─ [NEW] wanix-mesh         THE ONLY async crate. iroh Endpoint, the three ALPNs, the
               sync↔async Duplex bridge, MeshNode, MeshDialer, the inbound ProtocolHandlers,
               IrohCasStore, GossipPlumbPort. depends on wanix-9p-client + wanix-9p + wanix-id
               + wanix-cas + iroh + iroh-blobs + iroh-gossip + tokio.

wanix-cli  ── orchestrates: `wanix mount`, `wanix import`, `wanix cpu`, `wanix mesh-serve`.
```

**Why these boundaries, and where the subsystem designs disagreed:**

- The 9p-client, transport, import-export, cpu, and agent-mesh designs *all* independently converged on a new `wanix-9p-client` crate depending on `wanix-fs + wanix-protocol` only. **They are right and they agree.** This is the keystone. It is sync, transport-agnostic (generic over a `Duplex`), and contains zero network code — which is exactly what lets it be unit-tested over an in-memory pipe with no runtime.
- The designs split on whether transport/identity live in one `wanix-mesh` or several (`wanix-iroh`, `wanix-id`, `wanix-cas-iroh`). **Resolution: one async crate `wanix-mesh`** owns *all* iroh contact, because the three ALPNs share one `Endpoint` and one identity and one tokio runtime — splitting them just multiplies the async/sync bridge. But **identity and grants (`wanix-id`) and content-addressing (`wanix-cas`) get sync, iroh-free crates** so they are testable and reusable over pipe/TCP. The trust design correctly argued `wanix-id` shouldn't pull iroh; the data-plane design correctly argued `wanix-cas` core shouldn't either. Both hold.
- **`SubtreeFs` lives in `wanix-vfs`, not `wanix-id`.** The trust verdict caught this: re-rooting-at-a-subpath already exists in `Namespace::bind`'s source-subpath logic. The genuinely new part is the *rights gate*. Putting `SubtreeFs` in `wanix-vfs` (which already does re-rooting) and keeping only `GrantTable`/`Grant`/`AttachPolicy` in `wanix-id` avoids duplicating vfs logic and keeps `wanix-id`'s dependency story honest.

### 2.1 The mandatory precondition: lift shared constants into `wanix-protocol`

Verified: `errno_for_fs` (error.rs:45) is `pub(crate)` in `wanix-9p`; `O_RDONLY/O_WRONLY/O_RDWR/O_TRUNC/O_APPEND` (lib.rs:54–59) are private; `attr_for_metadata`, `DT_DIR/DT_REG/DT_LNK`, `P9_MODE_*` (attrs.rs) are `pub(super)`. A crate depending on `wanix-fs + wanix-protocol` **literally cannot compile against them.** So before the client crate can exist, these move to `wanix-protocol` as the single source of truth both server and client share:

- `O_*` open flags + `open_options_from_flags`/`open_flags_for` (the client needs the inverse).
- `DT_*`, `P9_MODE_*`, `attr_for_metadata`/`metadata_from_attr` (the client inverts attr→Metadata).
- `RREAD_HEADER_LEN=11`, `RWRITE` overhead, `RLOPEN_OVERHEAD=24` (for msize-correct chunking).

**`errno_for_fs` is NOT lifted as "the inverse," because it is non-injective** — verified: `EINVAL` ← `InvalidPath | InvalidOffset | InvalidTime | Other` (error.rs:47,56). The client defines its *own* canonical, documented, lossy `errno → FsError` table (`2→NotFound, 13→PermissionDenied, 21→IsDirectory, 20→NotDirectory, 17→AlreadyExists, 39→NotEmpty, 9→InvalidFd, 95→NotSupported, 22→Other, _→Other`). Three subsystem designs claimed an "exact inverse"; that claim is dropped.

---

## 3. The single load-bearing first build: the 9P client `FileSystem`

Everything in this document funnels through one struct. Build it first, build it over a pipe, and the entire mesh becomes incremental.

**`wanix-9p-client` — `P9Client` + `RemoteFs: FileSystem` + `RemoteFile: File`.**

It is the exact mirror of `serve_stream`: where the server decodes T-messages and encodes R-messages, the client encodes T with the existing `p9_t*` builders and decodes R with the existing `p9_decode_r*` decoders. The build is a state machine over codecs that already exist, not new wire code.

**Module layout (each < 250 lines per guardrails):**

| module | contents |
|---|---|
| `lib.rs` | `RemoteFs` (the `FileSystem` impl), `connect`/`attach` constructors |
| `transport.rs` | `Duplex: Read + Write + Send`; framed read helper reusing `P9FrameBuffer` |
| `conn.rs` | `P9Conn`: owns the `Duplex`, frame buffer, negotiated `msize`/`google_version`, `FidPool`, `TagPool`, and the `rpc()` blocking request/response primitive |
| `fid.rs` | `FidPool`/`TagPool` with free-lists (so a long-lived mount can't wrap `u32`); `ScratchFid` RAII guard that **best-effort `Tclunk`s on drop, even on panic** |
| `walk.rs` | `walk_to(path) -> ScratchFid` (clone-walk from root fid 0, chunked to ≤16 names/MAXWELEM); `walkgetattr_to` when `google_version >= 2` |
| `attr.rs` | `metadata_from_attr` (inverse of `attr_for_metadata`), `open_flags_for(OpenOptions) -> u32` |
| `error.rs` | the canonical lossy `errno → FsError` table |
| `file.rs` | `RemoteFile`: open fid + iounit + locally-tracked offset; clunks on drop |
| `readdir.rs` | `Treaddir` cookie-loop paging, **bounded** (max entries, max iterations) |

**The concurrency model — correct against *this* server.** `serve_stream` is strictly serial (verified above). So the client uses **one-outstanding-request guarded by `Mutex<P9Conn>`**: `rpc(tag, frame)` writes the frame, then blocking-reads frames until the matching tag returns. Concurrent callers serialize naturally onto the single stream. This is provably correct against a server that processes one frame at a time. A pipelined demuxer (background reader thread, per-tag oneshot map) is a *later* enhancement that does not touch the `FileSystem` methods — the `rpc()` seam is shaped so it can drop in.

**Five corrections baked into v1 (these are non-negotiable, from the adversarial verdicts):**

1. **Client-side frame-size ceiling.** `p9_declared_size` (frame.rs:193, verified) rejects only `size < P9_HEADER_LEN` — there is **no upper bound**. The server is fine because it clamps its *own* responses, but the whole point of import is consuming an *untrusted* peer's frames. A hostile server declaring a 4 GiB `Rread` OOMs the importer. The client's read loop **rejects any frame whose declared size exceeds the negotiated `msize`**, and poisons the connection. This is mandatory before importing any non-loopback peer.

2. **Honest seekability.** The server honors `Tread.offset` only when `file.is_seekable()` (io.rs:57, verified); for device/service files (`#term/data`, future `#kv`/`#agent` streaming files — *the things we most want to import*) it ignores the offset and reads sequentially. So `RemoteFile` **does not present a fictional local offset as truth.** It `Tgetattr`s on open to learn whether the remote is a regular file; for non-regular files it reports `is_seekable() = false`, forbids `seek()`, and reads purely sequentially. A local offset for a streamed file is a silent-corruption bug; we refuse it.

3. **RAII fid/tag guards, not "remember to clunk."** Every error path between `walk_to` and the operation leaks a fid into the server's `BTreeMap` (unbounded). `ScratchFid` clunks on drop including panic; `RemoteFile::drop` clunks its open fid; the `TagPool` reclaims on drop.

4. **`read_dir` is honest about cost and consistency.** `DirEntry` requires a full `Metadata`, but `Rreaddir` carries only `{qid, offset, dirent_type, name}`. So when `google_version >= 2` the client uses **`Twalkgetattr`** to batch entries-with-attrs (this is the WAN-friendly path and we *require* it for `ls -l`-class operations); otherwise it synthesizes `Metadata` from `dirent_type` for listing-only and documents that sizes/modes are placeholder. The server re-lists from scratch each `Treaddir` and pages by skipping `cursor <= offset` (readdir.rs, verified) — so listings are **O(n²) over WAN and are best-effort snapshots, not atomic.** We document this; we do not pretend it is cheap.

5. **Append via the server, not client offset math.** For `O_APPEND` the client passes `O_APPEND` in `Tlopen` and lets the server seek `End(0)` per write (io.rs append handling, verified) rather than racing a `Tgetattr`-per-write.

**What it unlocks the moment it compiles (and this is everything):**

- `RemoteFs` is `Arc<dyn FileSystem> + Send + Sync`, so `Namespace::bind(remote_fs, ".", "n/<node>", …)` mounts a remote namespace at `/n/<node>` — **Plan 9 import, realized.** Any path under `/n/<node>` resolves through it.
- A task bound against that namespace (`Task::bind`, `allocate_root_with_namespace`) runs **against a remote world** with zero new task code — **Plan 9 cpu's import leg.**
- It is testable today, with no async, no iroh, over `std::io::pipe` or a loopback `TcpStream`, against the *existing* `P9Server` serving the *existing* host root + `#term` + `#task`. The first demo needs none of the mesh.

---

## 4. The build plan — big vertical slices, each ending in a real demo

> **Status (added after the build; the slices below are written in the original
> imperative voice).** What each slice became:
>
> | Slice | Status |
> | --- | --- |
> | 1 — 9P client over pipe/TCP | **COMPLETE** — `wanix-9p-client`, the `mount-*` verbs (`tcp://` arm) |
> | 2 — identity + grants + `SubtreeFs` | **COMPLETE** — `wanix-id` (`AttachPolicy`, `GrantTable`), `SubtreeFs` in `wanix-vfs` |
> | 3 — iroh transport | **COMPLETE, then superseded in part** — `wanix-mesh` ships, but the Wanix↔Wanix plane is now the native `wanix-mesh-wire` (see the update note at the top); 9P stays the foreign edge |
> | 4 — `#kv` over the mesh | **COMPLETE** — `wanix-kv`, imports across the mesh |
> | 5 — CAS data plane + capsules | **COMPLETE** — `wanix-cas` (`LocalCasStore`, `#cas`), `IrohCasStore` in `wanix-mesh`, `wanix capsule` |
> | 6 — `#cpu` exec plane | **COMPLETE** — `wanix-cpu` + `CpuAcceptor`/`CpuDialer` in `wanix-mesh`, the `wanix-rust cpu` dial verb, and `mesh-serve --cpu` serving the acceptor behind the exec gate (refused on the public endpoint; `--peer`-scoped on the local one) |
> | 7 — plumber + agents on the mesh | **COMPLETE** — `wanix-plumb` + `GossipPlumbPort`, the `#agent` device with `RouterEngine`/`RemoteEngine` |

Each slice is a substantial, externally-visible capability that ends in a runnable demo. The ordering follows the verdicts' unanimous advice: **prove the sync control plane locally first; introduce async/iroh once; defer cpu-exec and agent-export until streaming-output plumbing and a grant/jail layer exist.**

### Slice 1 — The client primitive, over a pipe and TCP

Lift the shared constants (§2.1). Build `wanix-9p-client` with all five corrections. Wire `connect_stdio` (pipe) and `connect_tcp` (`TcpStream::try_clone`, exactly like `p9_listen/runtime.rs:127`). Add `wanix mount tcp://host:port /n/x` and a round-trip test harness that serves the existing host-root namespace from one thread and drives `RemoteFs` from another.

> **DEMO: "Loopback import."** `wanix serve` exports a directory over TCP; a second process `wanix mount`s it at `/n/local`, then `ls /n/local`, `cat /n/local/file`, `echo hi > /n/local/new`, `mkdir /n/local/d` — all served by the real `P9Server`, all driven by the new client. `#term` and `#task` reachable over the wire (the `#`-device walk is *not* filtered). Bytes are identical to a local `MemFs`.

### Slice 2 — Identity and the bolt-on grant boundary

Build `wanix-id`: `NodeIdentity` (ed25519, persisted 0600 at `~/.wanix/node.key`, stable across restarts), `GrantTable` (`Arc<RwLock<Vec<Grant>>>`, **default-deny**), `Authorization`, and an `AttachPolicy` trait. Build `SubtreeFs` in `wanix-vfs` (re-root + `Rights` gate, with `PermissionDenied` on every mutator unless the bit is set; **enforce centrally, one `require(right)` per method**, and a test that every mutation method on a read-only `SubtreeFs` returns `PermissionDenied`). Add `P9Server::with_policy(default_root, peer, policy)` — when policy is `None`, behavior is byte-for-byte today's. `handle_attach` consults `evaluate(peer, aname)` and installs the returned scoped root for that attach. Store `self.root = authorization.root` at attach (single-attach-per-connection is the v1 simplification; multi-attach scoping is a fid-namespace change deferred to a later slice).

> **DEMO: "Capability is a bind."** Over TCP (peer identity supplied explicitly, since there's no QUIC yet), a server grants peer K read-write to `projects/foo` and read-only to `docs`. Importer attaches `aname=projects/foo` → gets a `SubtreeFs` whose `.` is `projects/foo`; a write to `docs` returns EACCES; a walk can never escape the prefix. `rm` the grant file → next attach denied. The capability table is itself a filesystem you `cat` and `echo` to.

### Slice 3 — iroh transport: the mesh goes real

Build `wanix-mesh`. **Pin one wave and validate every signature against it before leaning on it** (the verdicts caught fictional versions and method names across designs): `iroh` ~0.98.x, `iroh-blobs` ~0.97 (**not** 0.102; self-described "not production quality" — treat the bulk plane as experimental), `iroh-gossip` ~0.96 (**not** 0.100). Use the *real* API: `Endpoint::builder(presets::N0).secret_key(sk).alpns(...).bind().await`; `endpoint.id() -> EndpointId` (**not** `node_id()`); `Connection::remote_id() -> EndpointId` (**not** `remote_node_id()`; handle the 0-RTT `Result` case or disable 0-RTT so identity is verified before any Tattach); `Router::builder(ep).accept(ALPN, handler).spawn()`; `accept_bi`/`open_bi`. ALPN `b"wanix/9p/1"`.

The **sync↔async bridge — designed, not asserted** (every verdict flagged this as *the* risk):

- **Inbound:** the `ProtocolHandler::accept(conn)` reads `remote_id()`, checks the `GrantTable`, then for each `accept_bi()` bi-stream does `tokio::task::spawn_blocking(move || P9Server::with_policy(scoped_root, peer, grants).serve_stream(BlockingRecv, BlockingSend))`. `serve_stream` runs **unchanged**.
- **Outbound:** `MeshDialer::dial` runs `connect`/`open_bi` on the mesh runtime, wraps the streams in a `BlockingDuplex` that drives `SendStream`/`RecvStream` via a **dedicated runtime `Handle`** (never `Handle::current().block_on` from a runtime worker — that panics; this concrete hazard was hand-waved in four designs and is fixed here), and hands the `Duplex` to `RemoteFs`. `FileSystem` calls run on non-runtime OS threads or `spawn_blocking` threads only.
- **First-write gotcha:** `open_bi` does not surface to the peer's `accept_bi` until the first byte is written. The 9P `Tversion` handshake writes immediately, satisfying this — but we assert it (a dialer that waited to read first would hang).
- **Bounded concurrency:** every live session pins one blocking-pool thread inside `block_on(recv.read())` for the session's lifetime. Default pool is 512. We **state this as a hard cap and size it**; "cheap many bi-streams" is misleading for long-lived mounts, so per-op timeouts (below) and a max-concurrent-session bound close the slow-peer DoS.
- **Resilience:** a `P9Client` wraps one bi-stream and dies on QUIC drop (routine over NAT/relay churn). v1 contract is **"best-effort, fails closed":** on drop, `/n/<node>` returns a clear `FsError` and open fids are invalid; a supervisor redials with backoff, re-attaches, and rebuilds root fid; the fid cache is invalidated wholesale on any reset. Connect retries with backoff because `online().await` does not guarantee dialability for ~2s (iroh #3713), and prefers an `EndpointTicket` (direct addrs) for first contact over bare-id dial. `rpc()` carries a **deadline**; on timeout it tears down the session rather than parking a thread forever (since `Tflush` is a no-op stub on this server, real cancellation = stream teardown — corrected from designs that claimed `Tflush` cancels).

> **DEMO: "Two laptops, one namespace, across the internet."** Node A on home NAT prints its NodeID/ticket. Node B on a coffee-shop NAT runs `wanix mount <A-ticket> /n/A`. `ls /n/A`, edit a file under `/n/A/projects/foo` (write lands on A), `cat /n/A/#term/...`. iroh handled NAT traversal + relay. Identity verified cryptographically; grants enforced. This is import/export on the real internet.

### Slice 4 — Service devices on the mesh: `#kv` and the imported service

Build the first *new* service device this branch lacks: `#kv` (a `FileSystem`, modeled on the `#task`/`#term` device shape that exists). Bind it into the served namespace alongside `#term`/`#task`. Because it's a `FileSystem`, it imports for free: `/n/A/#kv/<key>` over the mesh *just works* through the client built in Slice 1. This is the first proof that **service files cross nodes**, and it's where the seekability correction (Slice 1, #2) earns its keep — `#kv` value files are streaming, not seekable.

> **DEMO: "An agent reads a remote key store."** Node B binds `/n/A` and runs a qjs task whose namespace includes `/n/A/#kv`. The task `read`s `/n/A/#kv/config` and `write`s `/n/A/#kv/result` — operating Node A's key store as ordinary files, over QUIC, with no special-cased code. Mount-there, run-here.

### Slice 5 — The data plane: content-addressed blobs (venti)

Build `wanix-cas` (sync core): `ContentHash` (BLAKE3 newtype, lives in `wanix-fs` to avoid a cycle), `ContentStore` trait, `LocalCasStore` reusing `wanix-module-cache`'s audited owner-private/atomic-write/fd-verified boundary verbatim. Add the `#cas` device (`#cas/<hash>` read-only, `#cas/ingest` write-then-read-hash, `#cas/ticket/<hash>`, `#cas/have/<hash>`). In `wanix-mesh`, build `IrohCasStore` with the **correct** API (the data-plane verdict caught the fantasy method names): fetch is `let dl = store.downloader(&endpoint); dl.download(hash, Some(peer)).await` — **not** `blobs().download(...)`; local read is `blobs().get_bytes(hash)` / `blobs().reader(hash)` — **not** `blobs().get(...)`. Register `iroh-blobs::ALPN` on the same `Router` as the 9P ALPN — one endpoint, one identity, two planes.

Reframe capsules onto CAS: each world file → one blob (auto-dedup), a deterministic sorted `WorldManifest` → a blob whose hash *is* the capsule id, the collection → a `HashSeq`, the share form → a `BlobTicket`. **`WorldManifest::materialize` re-implements the `is_safe_relative` guard** (reject `..`/absolute/escaping paths and symlinks) and **caps blob size + HashSeq fan-out** — `get_bytes` loads whole blobs into memory, so one malicious ticket could OOM/disk-fill; the data-plane verdict refused to punt this to the trust layer, and so do we.

**The control/data split, made real and honest:** the *only* place 9P offloads to blobs is via a `FileSystem::content_hash(path) -> Option<ContentHash>` default-`None` hook surfaced through `Twalkgetattr`. When a CAS-aware client sees a hash and `len > 256 KiB`, it skips the `Tread` loop and fetches the blob from the data plane, BLAKE3-verified end-to-end. **We do NOT append a trailer to `Rgetattr`** — the data-plane verdict proved this codebase's own `p9_decode_rgetattr` calls `cursor.finish()` which errors on trailing bytes, and version negotiation echoes a fixed string. The hash rides as a real `cas.hash` xattr (a genuine synthetic `File` serving 64 hex bytes) or as a properly-versioned distinct field, never as raw appended bytes. The freshness guard ("no hash while a write fid is open") is **not** implemented via fid-mode scanning (`FidEntry` stores no mode — verified); instead `CasFs` invalidates on open-for-write and hash-on-close.

> **DEMO: "Ship a world by ticket."** `wanix capsule save ./world` prints a `BlobTicket`. On another node, `wanix capsule load <ticket>` fetches the HashSeq, verifies every blob, and materializes the world — shared `/bin/init` and `node_modules` deduped automatically. A 50 MB rootfs moves peer-to-peer on the blob plane, not through the 9P window.

### Slice 6 — cpu: send the agent to the data

Now the powerful move. Build `wanix-task`'s `#cpu` acceptor device. It reuses the *exact* local pattern (`allocate_root` → `task.bind(world, ".", ".")` → `configure` → `start`), generalized to a network: the caller dials, opens a control bi-stream and an **export** bi-stream, **writes a 1-byte role discriminator on each immediately after `open_bi`** (control=0, export=1 — the cpu verdict proved positional "first stream = control" is a QUIC race; stream order follows first-write, not open order). On the export stream the caller runs its *own* `P9Server` over a **scoped sub-namespace** (the job's working subtree + explicitly granted services, **read-only by default** — *not* `services_namespace_for_root`'s whole host root, which the cpu verdict showed is a remote-root hole with client-controlled symlink following). Node Y runs a `RemoteFs` over the export stream as the task's world and runs the task there — compute travels to data, or data is imported into the agent's world, your choice.

**Streaming output is real new plumbing, not assumed** (the cpu verdict's central break): `TaskDriver::start` runs the guest to completion and writes stdout to a `MemFs` buffer *after* eval. v1 therefore **delivers stdout/stderr/exit as a batch after `start()` returns** via `CpuEvent` frames on the control stream — honest about the current task model. Incremental streaming (a streaming-stdout `File` that pushes to the control stream) is a named, scoped follow-up, not hand-waved as already working. Cancellation: `TaskDriver` has no abort hook, so `CpuEvent::Cancel` stops *draining*, not the remote computation — documented, not pretended.

> **DEMO: "cpu a build onto the data node."** `wanix cpu --node <DATA> -- qjs build.js`. The build runs *on* the node holding the source tree, against its local fast namespace, writing outputs back through the reverse-export 9P session to the caller — or, with `--world-ref <hash>`, fetching a frozen world from the blob plane first. Exit status and output return on the control stream. Plan 9's cpu(1), over the internet, with a default jail.

### Slice 7 — Agents on the mesh, and the plumber

Build `wanix-plumb`: a `#plumb/<topic>/{send,recv}` device (modeled on the pipe-channel shape) whose backing port is a `GossipPlumbPort` mapping topic → `TopicId::from_bytes(blake3(topic))`, bridging `send → broadcast` and `recv ← Event::Received`. Newline-JSON envelopes `{kind, from, to, body}`; best-effort epidemic delivery, explicitly **not** a durable queue (durable handoff goes through `#kv` or a capsule blob). Then build the agent layer *on the mesh* (it doesn't exist on this branch yet): an `#agent` device and a `RouterEngine` that dispatches by session to a local engine or a `RemoteEngine` proxying to `/n/<node>/#agent`. **Heads-up baked in from the agent-mesh verdict:** the imported-`#agent` streaming-read path *deadlocks* the serial `serve_stream` (a `Tread` on a blocking `events` file freezes the whole connection). So imported agent event/reply streams get **one QUIC bidi stream per blocking open-file**, not one per attach — a blocking read stalls only its own stream. And **`#agent`/`#cpu`/`#task` export stays local-trust / grant-allowlisted only** until public auth lands; we do not export remote code execution to arbitrary NodeIDs (AGENTS.md flags public auth as unimplemented; we honor that).

> **DEMO: "Two agents, two machines, one conversation."** An agent on Node B imports `/n/A`, edits A's confined world, posts a `task.done` to `#plumb/build`; an agent on Node A `cat`s `#plumb/build/recv`, picks up the handoff, and `cpu`s a sub-agent onto Node C to author a bootable VM root, synced back as a capsule blob. Agents operating a Plan 9 mesh as files — the thing humans had the mechanisms for but never the patience to wire.

---

## 5. Hard risks, and how the architecture answers each

| Risk | The answer, baked in |
|---|---|
| **Async/sync bridge deadlock** (every verdict's #1) | One dedicated tokio runtime in `wanix-mesh`. Inbound: `spawn_blocking` runs unchanged sync `serve_stream`. Outbound: `BlockingDuplex` uses a held runtime `Handle`, never `Handle::current().block_on` on a worker (panics). `FileSystem` calls only on non-runtime threads. Per-session blocking-thread cost stated as a hard cap (512 pool) + per-op deadline + max-session bound. The `#agent` streaming-read head-of-line freeze is dodged by one-bidi-stream-per-blocking-open-file. |
| **WAN chattiness** (9P is per-op walk+open+read = 3–5 serial RTTs) | Honest, multi-pronged: `Twalkgetattr` collapses walk+stat to 1 RTT and is *required* for listings; CAS offloads bulk reads to the blob plane (control references hash, data moves P2P); `read_dir` is documented O(n²)/non-atomic; and the real structural answer is **cpu** — when chattiness dominates, send the compute to the data instead of pulling data over 9P. No claim that 9P metadata is "already pipelined" — the server is verified strictly serial. |
| **Malicious peer** | Two directions. Hostile *server* (the import threat): client frame-size ceiling ≤ msize (no OOM), bounded `read_dir`/walk-depth/iterations, per-op timeout, opaque symlink targets (no auto-resolve into the local namespace). Hostile *client* (the export threat): `NormalizedPath` already blocks `..` escapes; `SubtreeFs` re-roots + rights-gates; default-deny `GrantTable` keyed by verified `EndpointId`; cpu export is a **scoped read-only sub-namespace**, never the whole host root; `#task`/`#cpu`/`#agent` stay allowlisted until public auth lands. |
| **iroh churn** (renamed APIs, fictional versions, pre-1.0 blobs) | All iroh types confined to `wanix-mesh`; upward APIs are iroh-free (`Arc<dyn FileSystem>`, `EndpointId` as raw bytes). Pin one validated wave; verify *every* signature against the pin before use (the designs cited `node_id()`, `remote_node_id()`, `blobs().download()`, `blobs().get()`, and versions 0.102/0.100 that don't exist — corrected to `id()`, `remote_id()`, `downloader().download()`, `get_bytes()`/`reader()`, ~0.97/~0.96). Treat the blob plane as experimental (upstream says so). Share the ed25519 key as raw 32 bytes via `SecretKey::from_bytes`/`to_bytes` + a round-trip test — do **not** assume iroh re-exports `ed25519-dalek` (it's a private pre-release). |
| **Connection loss / no reconnect** | v1 contract is explicit: `/n/<node>` **fails closed** on drop (clear `FsError`, fids invalid), a supervisor redials with backoff and re-attaches, fid cache invalidated wholesale on reset. Connect retries because `online()` doesn't guarantee dialability for ~2s; prefer ticket (direct addrs) for first contact. |
| **Fid/tag exhaustion & leaks** | `ScratchFid`/`RemoteFile` RAII clunk on drop *including panic*; `FidPool`/`TagPool` free-lists so a long-lived mount can't wrap `u32`/`u16`; server fids die with the per-connection `P9Server` on stream EOF. |

---

## 6. The Plan 9 lineage, made explicit

Wanix has always been Plan 9 reincarnated; the mesh finishes the inheritance. Each piece descends from a named Plan 9 mechanism — and the twist is that **agents are the operators these mechanisms always needed.** Humans found per-process namespaces, import/export, and the plumber too fiddly to live in daily; an LLM that operates a confined world *as files* finds them native.

- **9P** → the wire is unchanged 9P2000.L. The mesh adds only the missing half: Wanix had the *server* (`serve_stream`); we build the *client* (`RemoteFs`). Same frames, same `msize`, same fids — now over QUIC.
- **`import`/`export` and `/n/`** → `Namespace::bind(RemoteFs, ".", "n/<node>")`. A remote namespace mounted at `/n/<node>` is import(4) verbatim; the longest-prefix resolver is the union mount. *Agents make it work where humans didn't:* an agent doesn't mind that `/n/alice/#kv/config` is three RTTs away — it just reads the file.
- **`cpu`** → Slice 6. A reverse-exported caller namespace + remote `allocate_root`+`bind`+`start` is cpu(1)'s "run there, namespace from here." The agent extends it: *send the agent to the data*, or *import the data into the agent's world* — same primitive, either direction.
- **factotum / secstore** → `wanix-id` + `#grant`. Identity is the ed25519 NodeID (iroh did the hard part: the pubkey is the address). Authorization is a default-deny capability table keyed by verified peer, and a grant *is a bind* (`SubtreeFs`). Factotum-lite: no in-band `Tauth` because the QUIC handshake already proved the key. The capability table is itself a filesystem an agent edits and revokes.
- **venti** → `wanix-cas` + iroh-blobs. BLAKE3 content-addressing is venti's archival store: worlds, capsules, and module inputs are immutable, deduplicated, hash-named blobs. A `BlobTicket` is a venti score that also says where to fetch it. The control plane references content by hash; the bytes move on the data plane, verified.
- **plumber** → `#plumb` + iroh-gossip. Typed messages routed between agents and tools by `kind`, across nodes, best-effort — the coordination bus that lets a mesh of agents hand work off without a central broker.

The through-line: **every remote thing is a file reachable at `/n/<node>/...`, and the only new core primitive is the `FileSystem` that speaks 9P to a remote server.** Build that one struct over a pipe first (Slice 1), and Plan 9's most ambitious ideas — finally workable on the real internet, finally operated by something with the patience to use them — fall out as composition.

---

**First commit to write:** lift the shared constants into `wanix-protocol` (§2.1), then `wanix-9p-client` with the five corrections, demoed over a loopback `TcpStream` against the existing `P9Server`. No async, no iroh, no new devices. That is the keystone; everything else is a vertical slice on top of it.