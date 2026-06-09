# ADR 0004: FileSystem Contract, Native Mesh Wire, and the 9P Edge Gateway

## Status

**Accepted — supersedes the prior "9P is the wire across all transports"
stance.** The direction below is decided, and the native mesh wire is now
**implemented**: the mesh's default Wanix↔Wanix path is the hand-rolled
FileSystem-over-iroh wire (`wanix-mesh-wire` bound to QUIC on `WANIX_FS_ALPN` by
`wanix-mesh`), not 9P over iroh. The 9P import path (`wanix-9p-client::RemoteFs`)
remains available and is the foreign edge. Implementation status lives in tests,
examples, and commit messages, not here — see `crates/wanix-mesh-wire` (codec +
client/server) and `crates/wanix-mesh/tests/mesh_native*.rs` (real-QUIC round
trip, differential-vs-9P, per-principal identity).

**Wire encoding decided — the hand-rolled fallback, not `irpc`.** §Decision below
names `irpc` as primary with "a hand-rolled minimal frame over raw QUIC streams"
as the explicit fallback "if the open-file/streaming mapping proves awkward." We
take the fallback, deliberately, judged through the project's *simple-not-easy*
mandate. The deciding facts: (1) `irpc`'s turnkey server loop discards the
ed25519 `remote_id()` and takes a bare `noq::Connection`, so the **per-principal
identity** requirement forces a hand-rolled accept loop anyway — `irpc`'s
headline value is the part we would throw away; and (2) the hand-rolled frame
reuses the **already-shipped** sync `Duplex` boundary + `BlockingDuplex` bridge
(`wanix-9p-client::transport`, `wanix-mesh::duplex`) verbatim with zero new
bridge code, whereas `irpc` would layer a second async-over-sync seam onto the
sync `FileSystem` boundary. The precedent already ships twice
(`wanix-cpu/src/wire.rs`, `event.rs`): typed structs, length-prefixed framing,
bounded decode. The honest cost is ~100–150 LOC of framing/dispatch boilerplate
`irpc`'s derive would generate — the "not easy" tax, accepted for fewer
interleaved concerns, total wire control, and near-zero `irpc`/`n0` pre-release
lock-in. Both options were compile-spiked against the locked versions
(`iroh =1.0.0-rc.1`, `irpc =0.16.0`); both built, so the tie-break is the
simplicity weighting, not a build failure.

The full wire contract and the op-by-op, phased implementation plan live in
**[docs/design/native-mesh-wire.md](../design/native-mesh-wire.md)**.

## Context

The root contract is the **`FileSystem` / `NamespaceOps` trait** in `wanix-fs` /
`wanix-vfs`, not any wire protocol. 9P is one *encoding* of that contract:
qids/fids/tags/msize never appear in the trait or in any service device, and the
9P stack (`wanix-protocol` + `wanix-9p` + `wanix-9p-client`) is a swappable codec
that ~90% of the workspace never sees.

Treating 9P as the *universal* wire was a mistake once the mesh moved onto iroh
QUIC. Between two Wanix nodes — both of which speak the trait natively — 9P is a
middle layer that buys nothing and costs:

- **chattiness**: `walk → open → read` is a round trip each, and a read costs
  ~2× latency; this is documented WAN pain, not a micro-optimization;
- **single-stream head-of-line blocking**: a never-EOF read (`#agent/events`,
  `#plumb/recv`) parks the whole import, which the codebase already works around
  by hanging dedicated QUIC streams off those files (`StreamingImportFs`);
- **tags** (a request-correlation key needed only because many requests share
  one ordered stream — redundant when a QUIC stream *is* the transaction); and
- **`msize` negotiation** (redundant with QUIC framing + flow control).

iroh already provides what 9P-over-TCP lacked: many independent flow-controlled
streams *and* connection-authenticated ed25519 identity. The right move is to
separate the **interface** (the trait) from its **encodings**, and stop paying
9P's transport-era costs where there is no foreign peer to interoperate with.

9P remains genuinely required at one place: a peer that already speaks 9P — the
Linux kernel (`v9fs`), v86/QEMU virtio-9p, external 9P tooling, and the current
browser cockpit (`workbench/src/wanix/p9.ts`).

## Decision

Encodings of the one `FileSystem` / `NamespaceOps` contract are chosen per
boundary. Three zones:

**1. Local (same process/machine): no wire.** A `wasm`/`qjs`/shell task hitting
a local `FileSystem` is direct trait calls. 9P is never involved. (Already true;
stated so it is not re-litigated.)

**2. Wanix ↔ Wanix across the mesh: a native FileSystem-over-iroh wire (the
hand-rolled frame; see Status).** This is the primary internal mesh wire, and it
is implemented. The `FileSystem` / `NamespaceOps` contract is expressed as a
`postcard`-encoded, length-prefixed frame over iroh QUIC bidi streams:

- **one stream per call / per open file** → no tags, no head-of-line blocking;
- **QUIC flow control** → no `msize` negotiation (keep at most an `iounit`-style
  chunk hint);
- **typed operations and typed errors** (`FsError` travels as a typed
  `WireFsError`, not an `Rerror` string or an errno table); `postcard` encoding;
- **not chatty**: compound path-resolve + stat, bulk `readdir`, and streaming
  reads, so sequential I/O is not per-message round trips;
- **per-principal identity is built in**: iroh authenticates every connection by
  the caller's ed25519 pubkey, and the server binds that principal once per
  connection to a principal-scoped `FileSystem` view (the per-principal seam
  ADR 0006 needs for attribution/authorization) rather than retrofitting it onto
  9P's `attach`/`uname` path. The principal is never carried on the wire.

An open `File` is stateful (handle, seek, streaming), so `open()` returns a
handle and subsequent reads/writes reference it over the file's own stream — the
expected fid-shaped lifecycle, which is fine. The *operations* are deliberately
9P-shaped (they are file operations); only the *encoding* is native.

**3. Wanix ↔ foreign at the edge: 9P, as a gateway.** `wanix-protocol` and
`wanix-9p` remain the 9P codec/server, but 9P is demoted to a compatibility
gateway at the foreign edge — Linux `v9fs`, v86/QEMU, external 9P clients, and
the current cockpit — *generated from the same trait*. The existing 9P `serve`
paths keep running for the cockpit now; the Linux/VM 9P path is added when the
v86/QEMU workflow lands. The foreign-edge 9P contract is unchanged:

- explicit version negotiation and rejection of unsupported versions;
- fid lifecycle: attach, walk, open, create, read, write, clunk, error mapping;
- directory iteration with opaque cookies;
- metadata, statfs, permissions, size, timestamps, link, rename, remove, mkdir,
  append where the backing filesystem supports it;
- compatibility probes and selected extensions (e.g. `walkgetattr`) only when a
  client requires them;
- stdio/TCP/WebSocket/`serve` transports are adapters that preserve binary frame
  boundaries and keep diagnostics out of binary response streams.

**Both wires are thin codecs over the single `FileSystem` / `NamespaceOps`
contract — never parallel first-class architectures.** The trait is the source
of truth. Neither wire fakes auth, special-file, xattr, ownership, inode-link,
or device semantics the trait cannot actually provide; unsupported features
return deliberate errors.

## The cockpit edge: viewer, composing client, or node?

The question "should the browser cockpit speak 9P/WS or iroh/native?" is
downstream of a prior one: **does the browser client have native Wanix
capabilities, or does it always talk back to a host?** The wire falls out of
that choice. There are three positions on the spectrum:

1. **Pure viewer** — the browser holds no namespace; every operation is a remote
   call against one host's namespace. This is essentially today's cockpit
   browsing `wanix:/` over 9P/WS.
2. **Composing client** — the browser runs the *namespace layer* (`wanix-vfs`)
   locally and `bind`s remote hosts/peers into its **own** local namespace,
   delegating file ops to the remote `FileSystem`s. It can compose more than one
   host at once; a pure viewer cannot.
3. **Full node** — the browser runs the whole core in wasm: local namespace +
   local `wasm`/`qjs` tasks + local devices + a mesh endpoint. It can run compute
   locally and even export resources to the mesh. This is the original
   in-browser Wanix.

**Hard constraint:** browsers cannot open raw UDP/QUIC, so iroh from the browser
requires a relay or WebTransport — a browser mesh-node is relay-dependent and
second-class — whereas a WebSocket link to a local/nearby host is trivial and
fast. This asymmetry, plus the north star ("browser is a frontend/deployment
option, not the runtime foundation"), drives the decision:

- **Default: the cockpit is a viewer / composing client (positions 1–2) talking
  to a host that is the full node and the cockpit's gateway into the mesh.**
  Browser ↔ host over WS; host ↔ mesh over iroh QUIC. The cockpit reaches
  `/n/<peer>` *through* its host and is never itself an iroh endpoint. The host
  is the single trust anchor (it holds the ed25519 key; the browser holds none).
- **Wire:** a pure viewer (position 1) is well served by **9P over WS** — it is a
  single low-latency local link where 9P's WAN costs do not bite, and `p9.ts`
  already exists, so this is a legitimate stable foreign edge. A composing client
  (position 2) mounts remote `FileSystem`s, which is Wanix↔Wanix and therefore
  pulls toward the **native protocol over WS** (a TS or wasm client of the native
  wire). The cockpit need not move off 9P/WS until it becomes a composing client.
- **Browser-as-node (position 3) is kept as a distinct deployment mode** —
  "Wanix in the browser, no host," reached via relay/WebTransport — valuable for
  no-install/serverless use, but explicitly *a deployment option, not the
  cockpit's foundation*. It is the one case that genuinely needs iroh/native in
  the browser and browser-side key custody.

Open questions (tracked with the ADR 0006 trust work where they overlap):

- Does the cockpit need to mount more than one host at once? If yes, it must
  become a composing client (run `wanix-vfs` in the browser), which is the line
  between "9P/WS viewer" and "native client."
- Is "Wanix in the browser, no host" a product goal? If yes, position 3 needs
  browser identity/key custody (non-extractable WebCrypto?) and a relay/
  WebTransport path — and that browser key is weaker custody than a native host,
  which should bound what a browser node may export or be granted.
- Does the cockpit's WS edge stay 9P (stable, `p9.ts` exists) or adopt the
  native wire (unifies with the mesh, gains typed errors / per-principal
  identity)? This relates to ADR 0005's browser/workbench client scope.

## Consequences

The mesh stops paying 9P's chattiness, head-of-line, tag, and `msize` costs
between Wanix nodes, and gains typed errors, native streaming, and
per-principal identity as first-class properties of the wire instead of
retrofits. 9P interop is preserved exactly where it is needed (Linux/VM, external
tools, the current cockpit). The everything-is-a-file *interface* is unchanged on
both wires; what changes is that the mesh wire is typed and stream-native.

The cost is real: the native wire was new work (the new mesh codec crate
`wanix-mesh-wire`, plus the import-site swap from `RemoteFs` to the native
`NativeFs` at the mesh dial/mount sites in `wanix-mesh`/`wanix-cli`; core
filesystem, namespace, task, and device crates were untouched), and the project
now maintains two encodings. The standing discipline is that both stay thin
codecs over the one trait. The `tcp://` foreign-edge mount path stays on
`RemoteFs` (9P) deliberately — it is the foreign edge, not mesh.

The wire is the **hand-rolled frame**, not `irpc` (see Status). It is a
`postcard`-encoded, length-prefixed frame over the existing sync `Duplex`
boundary, defined in a transport-agnostic, async-free `wanix-mesh-wire` crate
(deps: `wanix-fs`, `wanix-vfs`, `serde`, `postcard` — no iroh/tokio/irpc);
`wanix-mesh` binds it to QUIC on a second ALPN (`b"wanix/fs/1"`) beside
`WANIX_9P_ALPN` and supplies the streams + held runtime `Handle` via the existing
`BlockingDuplex` bridge. One bidi QUIC stream per call (one-shot ops) or per open
file (stateful, streaming) — no tags, no `msize`; `FsError` travels as a typed
`WireFsError`, not an errno table; `StreamingImportFs` and its `StreamPredicate`
retire because every open file is on its own stream by construction; and the
per-principal identity is the existing `peer_id_for(remote_id()) →
AttachPolicy::evaluate → per-connection root` resolution (the same three hops 9P
uses, minus `Tattach`/`uname`), reusing `wanix-id`'s `AttachPolicy`/`GrantTable`
unchanged. Dropping `irpc` removes the `irpc`/`n0-error`/pre-release-`noq`
lock-in this ADR previously accepted as a risk. The fallback trigger is now
inverted: reconsider `irpc` only if the hand-rolled open-file streaming
sub-protocol (backpressure/EOF/mid-stream typed-error/clean drop) proves
materially harder to get correct than `irpc`'s typed bidi channels would be, and
then for that op alone — not the base wire.

Future work updates this ADR only when it changes the FileSystem contract, the
mesh wire model, the 9P edge contract, the authentication/trust boundary, or the
backing filesystem semantics. The per-principal identity seam, when built,
graduates into its own ADR alongside the ADR 0006 trust-boundary records.

## Open questions: native wire contract

Held-open decisions on the v1 native wire (§Decision zone 2). The wire is
implemented and green; these are deliberately reserved, not bugs. File:line refs
and the longer discussion live in
[docs/design/native-mesh-wire.md](../design/native-mesh-wire.md) and its review.

1. **Per-attach scoping: port 9P's `aname`, or choose a native scope-selection
   shape?** The v1 wire binds the per-*principal* root once from the verified
   `remote_id()` and resolves `AttachPolicy` at the empty attach name only
   (`dial_native` discards `aname`; the server uses `ROOT_ANAME = ""`). So a peer
   gets exactly one root, and 9P's per-*attach* sub-scoping — the same peer
   attaching different named subtrees in one connection, i.e. a
   `--grant ANAME:PREFIX:RIGHTS` keyed on a non-empty `aname` — is **not reachable
   over the native wire** (native-plane grants must key at the root attach name).
   Open: when sub-scoping is needed (the ADR 0006/0007 authorization layer), do we
   thread the client `aname` onto the wire 1:1 (additive; empty stays the default)
   or adopt a cleaner native shape (attach-then-bind, or a typed scope request)?
   Decide it *with* the authorization layer — operations are 9P-shaped, but we did
   not reflexively copy 9P's attach ceremony before knowing the native wire wants
   exactly that shape.

   **Convergence note (do these as ONE wire change):** three deferred questions
   land on the same attach message — (a) this per-attach scope selection, (b)
   ADR 0007's principal-aware resources (Q10), and (c) Layer-3 delegation
   (agent acts with an ephemeral key plus a cert chain to its node key, scoped
   and expiring — UCAN/biscuit-shaped). When any one of them is built, design
   the typed attach payload `{scope?, cert_chain?}` for all three, or the wire
   gets three rounds of surgery. Hedge taken now: any code or contract that
   models "the principal" must use a structured/opaque principal type — never a
   bare 32-byte pubkey — so attenuated principals (key + caveats) slot in
   without rototilling device-side privacy filters and quotas.

2. **In-band device EOF (`FileReply::Eof`): keep reserved, or drop as YAGNI?**
   The wire defines (and the client decodes) `FileReply::Eof` — an explicit
   in-band device close, distinct from regular-file `Chunk(empty)` and from the
   stream drop / half-close that signals close today — but **no server path emits
   it** (stream-drop already conveys device close). Open: remove it as speculative
   generality (one fewer concept — the simple default) or keep it reserved for a
   future device that needs in-band close distinct from a stream drop? No
   correctness impact either way.

3. **Bind the read chunk clamp to the advertised `iounit_hint`.** The server
   clamps a read to `min(max, MAX_CHUNK_LEN)`, identical to the intended
   `min(max, MAX_CHUNK_LEN, iounit_hint)` *only because* every open advertises
   `iounit_hint == MAX_CHUNK_LEN`. This is a latent coupling, not a live bug: if
   any open path ever advertises a smaller per-file `iounit_hint`, a client could
   pull a chunk larger than the advertised hint. Resolution: add the third clamp
   term so the read bound is decoupled from that invariant — cheap; do it before a
   device advertises a non-default hint.
