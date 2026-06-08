# Native Mesh Wire — Implementation Review

Adversarial review of the native FileSystem-over-iroh wire (`wanix-mesh-wire` +
its QUIC binding in `wanix-mesh`), diffed `a1fb339..HEAD` on branch
`cockpit-mesh-integration`. The design reference is
[native-mesh-wire.md](native-mesh-wire.md); the decision record is
[ADR 0004](../adrs/0004-rust-9p-protocol-and-server-contract.md).

**Overall verdict: SHIP-READY.** Every phase landed as the plan specifies, all
non-negotiable invariants hold with file:line evidence, and `just check` is green
(fmt, module-lines, clippy `-D warnings`, `cargo test --workspace --locked`). The
per-principal foundation is real and is ready for ADR 0007 to build on. Findings
below are two minor, non-blocking observations and a short human-finish list — no
correctness, security, or guardrail bug was found.

---

## Per-phase status

| Phase | Scope | Status | Evidence |
|---|---|---|---|
| P1 | wire crate skeleton + value/error mirrors, bounded framing | **DONE** | `crates/wanix-mesh-wire/src/{value,error,frame}.rs`; 19 unit tests pass |
| P2 | proto enums + sync server dispatch + sync client facade | **DONE** | `src/{proto,server,client,file}.rs`; `tests/round_trip.rs` (9 tests) |
| P3 | module hygiene + reusable conformance suite | **DONE** | all modules ≤224 non-test lines; `src/conformance.rs` + `tests/conformance.rs` |
| P4 | bind to QUIC in `wanix-mesh` (ALPN, handler, dialer, node) | **DONE** | `wire_handler.rs`, `dialer.rs`, `node.rs`; `tests/mesh_native.rs` (3 real-QUIC tests) |
| P5a | differential + per-principal identity proofs | **DONE** | `tests/mesh_native_differential.rs` (2), `tests/mesh_native_identity.rs` (3) |
| P5b | import-site swap; retire `streaming.rs`; device tests | **DONE** | `streaming.rs`/`streaming/predicate.rs` deleted; `ticket.rs`/`mount.rs` swapped; agent/plumb/blobs on native plane |
| docs | ADR 0004 reconcile, walkthrough, design status | **DONE** | ADR 0004 rewritten; `native-mesh-wire.md` status = implemented |

---

## 1. Simple-not-easy held — VERIFIED

The wire is the hand-rolled `postcard` + 4-byte-LE-length-prefix + one-stream-
per-transaction design. No `irpc`, `noq`, or `n0-error` leaked anywhere.

- `wanix-mesh-wire/Cargo.toml:13-17` — deps are exactly `wanix-fs`, `wanix-vfs`,
  `serde`, `postcard`. Nothing else.
- `grep -rEn 'iroh|tokio|irpc|noq|n0_error'` over `crates/wanix-mesh-wire/src/`
  matches **only doc-comment prose** (e.g. `lib.rs:8`, `client.rs:23`); zero
  `use`/`extern`/code references.
- It mirrors the `wanix-cpu` wire idiom: `frame.rs` is the same length-prefix +
  bounded-decode discipline as `wanix-cpu/src/wire.rs` and
  `wanix-9p-client/src/transport.rs:47` (named in `frame.rs:4-9`).
- Debuggable: every frame is a typed `postcard` body (`proto.rs`), bounded by
  explicit named ceilings (`frame.rs:22,29`), and a hang is one stream, not
  generated runtime code.

## 2. Guardrails — VERIFIED

- **Zero forbidden deps in the wire crate.** `cargo tree -p wanix-mesh-wire
  -e no-dev` resolves to exactly `postcard`, `serde`, `wanix-fs`, `wanix-vfs` —
  no iroh/tokio/irpc/noq in the *transitive* tree either.
- **tokio/iroh confined to `wanix-mesh`.** All async/iroh lives in
  `wire_handler.rs`, `dialer.rs`, `duplex.rs`, `node.rs`. The wire crate only
  ever sees the sync `Duplex` (`client.rs:34` `StreamFactory::open_stream ->
  Box<dyn Duplex>`); the mesh implements it via `IrohStreamFactory`
  (`dialer.rs:185`) over `handle.block_on(connection.open_bi())` + the existing
  `BlockingDuplex`. No new bridge concept was introduced.
- **Core crates UNTOUCHED.** `git diff --stat a1fb339..HEAD` over
  `wanix-fs/-vfs/-task/-kv/-plumb/-agent/-term/-cas/-pipe/-id/-9p/-9p-client/`
  `-protocol/-cpu` is **empty**. No `serde` derive was added to `wanix_fs`
  (`grep serde crates/wanix-fs/src/{error,metadata}.rs` is empty); the mirror
  lives entirely in `wanix-mesh-wire` (`value.rs`, `error.rs`).
- **9P foreign edge intact.** `wanix-9p`, `wanix-9p-client`, `wanix-protocol`
  unchanged. The `tcp://` mount path still builds a `RemoteFs` (9P) at
  `mount.rs:194 dial_tcp_remote`; the dialer keeps `dial`/`dial_attach` (9P) for
  the foreign edge beside `dial_native` (`dialer.rs:71,87,109`). Both ALPNs are
  advertised (`node.rs:439 endpoint_alpns -> [WANIX_9P_ALPN, WANIX_FS_ALPN]`), so
  a node speaks both wires during the transition.

## 3. Correctness — VERIFIED

- **Typed `WireFsError`, no errno round-trip.** `error.rs:58-94` is an
  exhaustive 1:1 `From` pair over all 12 `FsError` variants (no wildcard arm — a
  new `FsError` variant would fail to compile), and both `String`-carrying
  variants are preserved verbatim (`error.rs:130` test:
  `InvalidPath("a/../b")` and `Other("remote detail 42")` survive both
  conversions + a `postcard` round trip). The conformance proof asserts *variant
  identity*, not just `is_err`: a missing path is exactly `FsError::NotFound` and
  a non-empty `remove_dir` is exactly `FsError::NotEmpty`
  (`conformance.rs:276,287`), run against the native import.
- **Per-principal identity from `remote_id()`.** `wire_handler.rs:150` binds the
  principal once per connection from the cryptographically verified
  `connection.remote_id()` (never a client-claimed name), resolves the root via
  `AttachPolicy::evaluate(peer, "")` (`wire_handler.rs:101-108`), and
  default-denies on `None` by returning before any `accept_bi`
  (`wire_handler.rs:153-155`). The identity test proves the **same**
  `GrantTablePolicy` yields byte-identical scoped roots on the native and 9P
  planes for the same peer (`mesh_native_identity.rs:239`), and that an ungranted
  peer is refused (`mesh_native_identity.rs:186`, surfacing
  `FsError::Other` — a transport fault, not a typed app error, because a denied
  peer never reaches the backing FS).
- **Never-EOF streaming, own stream, no deadline, no `StreamingImportFs`.**
  `streaming.rs` and `streaming/predicate.rs` are deleted; the only remaining
  references are doc comments. The open-file loop reads the next `FileOp` with no
  deadline (`server.rs:240`; the `deadline` arg is discarded at the wire layer,
  `server.rs:59`), and the transport applies the deadline to writes only via the
  asymmetric `BlockingDuplex::with_deadlines(send, recv, handle, None /*read*/,
  deadline /*write*/)` (`wire_handler.rs:204`, `duplex.rs:160`). The head-of-line
  proof parks a never-EOF `#plumb/recv` read and asserts a concurrent sibling op
  completes in <2 s, then publishes and confirms the parked read wakes with the
  exact envelope (`mesh_native.rs:267-355`).
- **Bounded framing rejects oversized frames without allocating.**
  `frame.rs:140-146` rejects an over-ceiling declared length **before** the
  `vec![0u8; declared]` allocation at `frame.rs:147`. The test declares a 1 GiB
  frame with only the 4-byte prefix present and expects `TooLong`, not an OOM
  (`frame.rs:200`). The server-side `Chunk` cap is independent of the client's
  `Read{max}` (`server.rs:277`).
- **Dispatch on `spawn_blocking`, not on a runtime worker.**
  `wire_handler.rs:181` runs each stream's sync `serve_one` inside
  `tokio::task::spawn_blocking`, so a parked never-EOF read pins a blocking-pool
  thread and `BlockingDuplex::block_on` is safe (the documented runtime-worker
  hazard, `duplex.rs:17-26`). One `MAX_CONCURRENT_SESSIONS = 512` permit is held
  per live stream (`wire_handler.rs:168,181-185`; `node.rs:40`).

## 4. Differential proves native == 9P — VERIFIED

`mesh_native_differential.rs` serves the **same** `Arc<dyn FileSystem>` (a
`MemFs` + `#kv` namespace) over two nodes — one `serve_native`, one `serve` (9P)
— and runs the reusable conformance suite across both imports
(`mesh_native_differential.rs:87-114`). It honestly splits two claims: (1) the
full conformance suite passes against the native import (faithful encoding,
including the symlink/set_times/nofollow/full-readdir-metadata surface 9P never
bridged); (2) the **subset both planes bridge** runs against the native AND the
9P import of the same backing and both pass identically
(`differential_shared_surface`, `:150`). It deliberately excludes native-only
features (symlink, set_times, real readdir entry sizes) from the differential
because 9P genuinely does not carry them — asserting equivalence there would test
an intended divergence, not a regression. This is a correct, non-cheating
differential.

The import swap did not weaken any device test: `mesh_agent` (2), `mesh_plumb`
(3), `mesh_blobs` (2) keep their pre-swap `#[test]` counts and were re-pointed at
the native plane (`serve_native`, `dial_native`, `serve_native_with_blobs`,
`serve_native_with_plumb`).

## 5. `just check` — GREEN

- `fmt --check`: PASS (wire crate is in the Justfile fmt package list,
  `Justfile:4`).
- `module-lines`: PASS. The wire crate's largest non-test modules are
  `proto.rs` (224) and `server.rs` (223), both under the 250 warn limit; nothing
  new is over limit. The 4 warnings (`codex.rs`, `exec_server.rs`,
  `app.rs`, `sites/lib.rs`) are pre-existing baseline modules, unrelated to this
  work.
- `clippy --workspace --all-targets -D warnings`: PASS (clean).
- `cargo test --workspace --locked`: PASS. Wire crate: 19 unit + 2 conformance +
  9 round_trip. Mesh native: `mesh_native` (3), `mesh_native_differential` (2),
  `mesh_native_identity` (3), plus the device tests on the native plane. No
  failures.

`Cargo.lock` carries the `wanix-mesh-wire` entry (`Cargo.lock:5105`) and the
workspace `Cargo.toml` lists it as a member (`Cargo.toml:13`); `--locked` passes.

---

## Minor observations (non-blocking, not bugs)

1. **`FileReply::Eof` is defined and client-handled but never server-emitted.**
   `proto.rs:220` defines `Eof`; the client treats it as a 0-byte read
   (`file.rs:73`); a `proto.rs:351` test round-trips it. But the server's
   `read_op` only ever emits `Chunk(empty)` for regular-file EOF
   (`server.rs:278-280`) and never constructs `Eof`. A never-EOF device today
   signals close via the dropped stream / half-close, not an explicit `Eof`
   frame. This is harmless and forward-compatible (the variant reserves the
   explicit-device-close encoding for a future device that wants in-band EOF
   distinct from a stream drop), but it is currently produce-side dead. Either
   wire a device that emits it or note it as reserved; today it is the latter by
   omission.

2. **Chunk cap is a 2-way `min`, not the plan's 3-way.** `server.rs:277` clamps
   to `min(max, MAX_CHUNK_LEN)` rather than `min(max, MAX_CHUNK_LEN,
   iounit_hint)`. This is behaviorally identical because the server sets
   `iounit_hint == MAX_CHUNK_LEN` at open (`server.rs:227`), so the third term
   never binds. The code documents this (`server.rs:271-275`). If a future open
   path ever advertises a smaller per-file `iounit_hint`, the third clamp term
   must be added or a client could pull a chunk larger than the advertised hint.
   Add the `iounit_hint` term now (cheap, future-proof) or leave a guard comment
   at the `iounit_hint` assignment so the coupling is not lost.

## Human-finish list (none block ADR 0007)

- **Cockpit is not covered by the native wire** — by design (no browser QUIC).
  The typed-error/per-principal wins do not reach the cockpit until a TS/wasm
  native client or a host-side gateway exists. Stated in ADR 0004 and the design
  doc; flagged here so it is not assumed covered.
- **`dial_native_attach` does not yet carry `aname` on the wire**
  (`dialer.rs:131` discards it; v1 resolves at the empty `ROOT_ANAME`,
  `wire_handler.rs:51`). Per-attach scoping (distinct from per-principal scoping)
  is a future path; the seam is reserved, not built.
- Optionally fold the two minor observations above into the next cleanup cycle.

---

## Verdict on the per-principal foundation for ADR 0007

**Ready.** The per-principal seam is the non-no-op `NamespaceProvider` the queued
CLAUDE.md note asks for: the provider is the existing `AttachPolicy`, the
principal is the cryptographic `remote_id()` bound once per connection
(`wire_handler.rs:150`), and the resolved per-connection root is *actually used*
for the connection's whole life (`wire_handler.rs:153,171`) — no discarded result,
no dead abstraction. It is proven equivalent to the 9P plane's grant resolution
for the same peer (`mesh_native_identity.rs:239`) and default-denies the
ungranted peer (`:186`). A `#chat`/resource device served over this wire inherits
attribution (stamp posts with the authenticated pubkey), `who` (live
`remote_id()`s), and layer-2 ACLs (`GrantTable` grant/revoke) as policy problems
over the raw `PeerId` — the identity half is done. ADR 0007 can build on this as
designed.
