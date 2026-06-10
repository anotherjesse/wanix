# Cockpit Meets Mesh: One Unified Story

This document is the integration narrative for two parallel spikes that grew
in different directions on the Wanix tree, and the synthesis they collapse
into. It is not a build plan (see `docs/integration/plan.md` for the slices),
and it is not a use-case catalog (see `docs/recipes/` for those). It is the
story of what each spike proved, where they overlap, where they conflict, and
what the integrated system looks like once both halves are wired through one
identity, one namespace, and one wire.

The shortest possible summary: the `origin/rust` spike built a cockpit on top
of a single-node runtime and discovered that the file-shaped device model
could carry a real operator UI. The `cpu` spike built a mesh-native runtime
substrate and discovered that every Plan 9 primitive (`import`, `export`,
`cpu`, `plumber`, `venti`, `factotum`) is just another `FileSystem`. Each
spike found half of the same machine. This doc describes that machine and
how the two halves fit.

## 1. Two spikes, two halves of the same machine

### The origin/rust spike: cockpit-TS

The `origin/rust` branch took the workbench seriously as a product surface.
It built an operator cockpit on top of VS Code, in TypeScript, riding the
Go-served `globalThis.Wanix` bridge plus a direct-9P route. What that branch
gave us, concretely:

- `WanixBridge` (`workbench/src/web/bridge.ts`): a VS Code `FileSystemProvider`
  for `wanix:/`, mediating walk/read/write/mkdir/rm across the served root.
- `WanixSystemView` (`workbench/src/web/system-view.ts`): a sidebar tree
  modeling drivers, tasks, terminals, agents, and activity as live categories
  with one-second polling.
- `WanixServiceInspector` (`workbench/src/web/service-inspector.ts`): a
  `wanix-inspect:` URI handler that read-only renders `#task/<id>` and
  `#term/<id>` allocator/ctl/stream files, treating unsafe links (the
  allocator `new` file) as control-only.
- A family of demo modules, each a real activation flow rather than a static
  README: `qjs-starter`, `wasm-starter`, `duet-demo`, `http-app-demo`,
  `agent-repair-demo`, `v86-shared-demo`, plus the
  `cockpit-self-check` runtime probe and `agent-tool-contract` publisher.
- A ~40-command `extension.ts` that threaded everything: refresh, open
  inspector, install/run each demo, run self-check, open the tool contract.
- `workbench-assets.ts`: a small content registry for bundled fixtures
  (`rust-guest.wasm`, demo templates).

What the spike actually proved is that Wanix can have a cockpit at all:
a single VS Code-shaped surface where an operator browses the namespace,
inspects allocator devices safely, runs a multi-task demo end to end, and
gets back a green self-check report. The cockpit-TS spike showed that the
file-shaped device model is rich enough to power a real operator UI without
inventing a side-channel control protocol.

What it did not prove is that any of this scales beyond one node. Every
demo, every tool surface, every probe assumed the workbench was talking to
*its* Go server. The `bridge` was a `FileSystemProvider` over one mount;
the `system-view` polled one namespace; the `agent-repair-demo` carried its
own deterministic fixer (`repairQjsProgram`) because there was no real
agent device on the other end. The cockpit was a single-tenant operator UI
for a single-tenant runtime. It was also, structurally, *Go-flavored*: the
helpers it depended on came from a Go-served bridge, and the v86 boot path
it shipped was a MessagePort/CBOR bridge into an emulator running in the
same browser tab.

### The cpu spike: mesh-Rust

The `cpu` branch took the runtime seriously as a mesh. It built, in Rust,
the entire substrate that a mesh-native Wanix needs:

- `wanix-id` (`crates/wanix-id`): ed25519 `NodeIdentity` persisted at
  `~/.wanix/node.key`, a default-deny `GrantTable` keyed by verified peer,
  an `Authorization` shape, and an `AttachPolicy` trait the 9P server can
  consult at `Tattach` time. `SubtreeFs` re-roots the granted prefix and
  rights-gates every mutator.
- `wanix-mesh` (`crates/wanix-mesh`): the one async crate. It owns the iroh
  `Endpoint`, the three ALPNs (`wanix/9p/1`, `iroh-blobs`, `iroh-gossip`),
  the sync to async bridge for the existing synchronous `P9Server`, and the
  inbound `ProtocolHandler` that authenticates the peer before any
  `Tattach` runs.
- `wanix-9p-client` (`crates/wanix-9p-client`): the missing half of 9P. A
  synchronous `RemoteFs: FileSystem` that mirrors `serve_stream`'s codec,
  honest about seekability, RAII-clunking fids, with a per-frame
  size ceiling against hostile servers.
- The new file-shaped devices the cockpit-TS spike had to fake: `#agent`
  (`crates/wanix-agent`), `#kv` (`crates/wanix-kv`), `#pipe`
  (`crates/wanix-pipe`), `#plumb` (`crates/wanix-plumb`), `#cas`
  (`crates/wanix-cas`), `#cpu` (`crates/wanix-cpu`). Each follows the
  device patterns the existing `#task` and `#term` established: allocator
  (`new`), control (`ctl`), stream (`events`, `data`, `recv`, `send`),
  metadata (`info`, `status`).

What this spike proved is that every Plan 9 mesh primitive (`import`,
`export`, `cpu`, `plumber`, `venti`, `factotum`) can be expressed as a
`FileSystem` reachable through `Namespace::bind` at `/n/<node>/...`, with
exactly one new core primitive (a `FileSystem` that speaks 9P to a remote
server) and one new edge (QUIC transport plus ed25519 identity). The
mesh-Rust spike showed that the runtime is mesh-native end to end.

What it did not prove is that anyone can operate it. The cpu branch
shipped a stub `extension.ts`, a stub `bridge.ts`, and a 9P-over-WebSocket
console. There is no sidebar, no inspector, no demo flow, no self-check.
The cpu spike is a substrate looking for a surface.

The integration story writes itself: **the cockpit is the operator UI for
the mesh.** Cockpit-TS owned the surface; mesh-Rust owns the substrate;
the merge is one branch where you serve the cpu runtime and open the
origin cockpit against it, with every demo re-targeted at the new
devices and a mesh panel as a first-class category.

## 2. Where they overlap: TS module to Rust device

The two spikes overlap heavily on intent and almost not at all on
implementation. Most of the cockpit-TS demo logic was a deterministic
TypeScript simulation of a feature the mesh-Rust spike later built as a
real device. The integration is, mostly, *deleting the simulation and
pointing the surface at the real device*.

| Cockpit-TS module | Mesh-Rust device that subsumes its content | Note |
|---|---|---|
| `agent-repair-demo.ts` (with internal `repairQjsProgram` fixer) | `#agent` via `crates/wanix-agent` | The deterministic fixer is retired; the demo opens an `#agent/new` session, writes a prompt, streams events, applies the reply. The local `AgentEngine` drives the demo without a remote LLM; the same code path works against a `RemoteEngine` wrapping `/n/<peer>/#agent`. |
| `duet-demo.ts` (with `/duet/shared/*` plain-file handoff) | `#cas` via `crates/wanix-cas` plus `#plumb` via `crates/wanix-plumb` | Producer writes blob via `#cas/ingest`, publishes hash via `#plumb/duet/send`; transform reads via `#cas/<hash>` and ingests result; verify compares hashes. The shared-file dance is retired; the content-addressed exchange is the demo. |
| `qjs-starter.ts` (template install + qjs launch) | `#task` via `crates/wanix-task` with `crates/wanix-qjs` driver | Templates stay in TypeScript (editor UX); execution goes through `#task/new/qjs` with `cmd`/`env`/`dir` writes and `ctl start`. No `globalThis.Wanix` hooks. |
| `wasm-starter.ts` | `#task` via `crates/wanix-task` with `crates/wanix-wasm` driver | Same shape as qjs-starter; binary fetched via `workbench-assets.ts`, written to scratch path, executed via `#task/new/wasm`. |
| `http-app-demo.ts` (route catalog, preview, source) | `#task` plus `#plumb/http` plus a new `/.wanix/app/<name>` HTTP route on `wanix-cli` serve | The route resolves `/apps/<name>.{js,wasm}`, allocates a `#task`, binds request body as stdin, returns stdout with `x-wanix-task-id` headers, emits a `#plumb/http` event so the cockpit timeline updates without polling. |
| `cockpit-self-check.ts` (probes drivers + `#task` + `#term`) | All seven devices: `#task`, `#term`, `#agent`, `#kv`, `#pipe`, `#plumb`, `#cas`, plus `#cpu` local-trust gate | Probe list widens to cover every device; `#cpu` probe verifies it is *not* exported publicly. |
| `agent-tool-contract.ts` (8-tool surface) | Same tools, repointed at cpu paths | `readFile` to `wanix:/`, `runTask` to `#task/new/<kind>`, `observeTask` to `#task/<id>/fd/{1,2}`, `writeSystemSnapshot` to `#cas/ingest`; `previewHttpRoute` deferred until the HTTP route lands. |
| `system-view.ts` (drivers/tasks/terminals/agents categories) | Same categories plus `#agent`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#cpu`, and a `mesh` category | Each device category derives from typed discovery JSON rather than a hard-coded list; allocator `new` files are unsafe-link, never auto-read. |
| `service-inspector.ts` (`#task`/`#term` allocator/ctl/stream classifier) | Same patterns, extended to every new device | The allocator-vs-control-vs-stream classification already aligns with `crates/wanix-agent/src/files.rs:23-58`; no shape change, just wider coverage. |
| `bridge.ts` (VS Code `FileSystemProvider` over `wanix:/`) | The cpu `WanixP9Handle` plus `WanixHandle` (`workbench/src/wanix/`) | Bridge is restored on top of the existing async TS 9P client. No Go bridge involved. |
| `extension.ts` (40 VS Code commands) | Same commands, re-targeted at cpu devices and the mesh panel | Activation wires bridge plus system-view plus inspector plus mesh-panel; commands invoke devices via the file API, not a side channel. |
| `workbench-assets.ts` (bundled fixture registry) | Same module, unchanged | Still needed for `rust-guest.wasm` and demo templates. |
| `v86-shared-demo.ts` (v86 to browser `/shared/`) | Either `#cas` plus `#plumb` cross-VM, or flat 9P-shared directory | Default is to roll into the `duet`-style CAS plus plumb story; the open question is whether a v86 boot path specifically needs the flat-shared pattern. |
| direct-v86 MessagePort/CBOR bridge | Direct-9P route discovery (`/.well-known/wanix.json`) | Retired entirely per CLAUDE.md guardrail. |
| Go-served `globalThis.Wanix` helpers | `qjs:std`, `qjs:os`, service files | Retired entirely per CLAUDE.md guardrail. |

The pattern is consistent: every demo or probe the cockpit-TS spike had to
hand-roll because the substrate did not exist, the mesh-Rust spike built as
a file-shaped device. The merge is a re-pointing exercise on the demos and
a coverage expansion on the system view and inspector. The cockpit code
shrinks; the cockpit reach grows.

A second pattern worth naming: the cockpit-TS demos that survive intact are
exactly the ones whose work was always going to happen in the editor (template
install, command threading, sidebar refresh, asset bundling). The demos that
do *not* survive intact are the ones that simulated runtime behavior in
TypeScript because no runtime existed for them. After the merge, the editor
keeps the editor concerns and the runtime keeps the runtime concerns. The
seam between them is the 9P wire and the device file shape.

## 3. The one real conflict: trust and identity

Almost every overlap is a clean re-pointing. There is one place where the
two spikes carried genuinely incompatible models and only one survives.

**Cockpit-TS assumed one trusted process.** The bridge talked to *its*
Go server; the system-view polled *its* namespace; the inspector
classified files in *its* `#task`/`#term`. There was no identity. There
was no grant. The cockpit was the only operator, and the server was the
only world. "Trust" was scoped to "you launched both."

**Mesh-Rust assumes a default-deny capability table keyed by verified
peers.** `wanix-id` persists an ed25519 keypair; the QUIC handshake
authenticates the peer's pubkey; `P9Server::with_policy` consults the
`GrantTable` at `Tattach` time; the granted root is a `SubtreeFs`
re-rooted at the granted prefix with rights gates on every mutator.
Identity is the wire identity. A grant is literally a bind. `Tauth`
stays `ENOSYS` forever because there is nothing to authenticate in-band;
the QUIC handshake already proved the key.

These do not coexist by accident. If you naively port the cockpit onto
the mesh runtime, the cockpit becomes a single trusted operator running
against a server that thinks every connection needs explicit
authorization. The cockpit's `wanix.refresh` would fail half its probes;
demo flows would 403 against `#cpu` and `#agent`; mounting `/n/<peer>`
would silently no-op because no grant exists.

The resolution is to take `wanix-id` seriously at the cockpit layer, not
work around it:

1. **The cockpit is itself a peer.** When the cockpit attaches to the
   local server, it presents (for the trusted-loopback case) a session
   token issued by the local `serve` process. The server treats the
   cockpit's attach as an authenticated peer, not as a special operator.
   The cockpit therefore goes through exactly the same `AttachPolicy`
   path as any other peer, with the same `Tattach` semantics and the
   same `SubtreeFs` wrapping. There is no "cockpit attach" code path;
   there is only "peer attach" with the cockpit as one peer.
2. **The grant table is a first-class cockpit category.** The mesh panel
   surfaces the local `NodeIdentity`, the verified peer list, the
   `GrantTable` snapshot, and the `/n/<node>` mounts. Operators issue
   `wanix.mesh.dial`, `wanix.mesh.mount`, `wanix.mesh.revokeGrant` from
   VS Code; these are reads and writes against the grant-table device,
   not RPCs into a side channel. The grant table is itself a file. The
   operator sees grants by reading; they create grants by writing; they
   revoke grants by writing. Auditability comes for free because the
   grant device emits events on `#plumb/mesh/grants`.
3. **Default-deny stays default-deny.** A peer with no grant cannot
   mount anything. The cockpit shows that honestly: empty grant table,
   no `/n/<peer>` mounts visible, dial succeeds at the transport layer
   but `Tattach` returns EACCES until a grant is written. There is no
   secret "operator mode" that bypasses grants; if the cockpit cannot
   mount, neither can anyone else, because the cockpit *is* anyone else.
4. **`#cpu`, `#agent`, `#task` export stays local-trust until public
   auth lands.** This is the AGENTS.md trust-boundary commitment. The
   cockpit can *initiate* `#cpu` against a granted peer, but the mesh
   panel gates that behind an explicit prompt, and the server refuses
   `#cpu` export to non-loopback peers. The gating is in the device,
   not in the cockpit; the cockpit just renders the gate honestly.
5. **The session-token bootstrap is the seam, not the final answer.**
   For trusted-loopback browser-as-peer (the first cut in Slice 9), the
   cockpit's WebSocket attach carries a token the local serve issued.
   Full ed25519 plus iroh in the browser is deferred to a follow-on
   cycle, but the discovery flag and grant-table semantics are already
   in place. The seam is honest about its limits: the discovery JSON
   advertises `peer_auth: "loopback-token"` and the mesh panel surfaces
   that fact to the operator.

The conflict is real and the resolution is asymmetric: the mesh-Rust
trust model wins because it is sound across nodes, and the cockpit-TS
single-trust assumption is the local-trust special case of it. Identity
and grants are not bolted onto the cockpit; the cockpit is rebuilt on
top of them.

A useful frame: in the cockpit-TS world, "what can I see?" was a
question with one answer (everything the server has). In the merged
world, "what can I see?" is a question whose answer is whatever the
grant table says, and the grant table is itself something the operator
can read. The cockpit gets *less* implicit power and *more* explicit
power, which is what a real operator UI looks like.

## 4. The browser-as-peer model

The mesh-Rust spike makes "what is the browser, exactly?" answerable
for the first time. It is a peer. Specifically:

**What the browser hosts locally.**

- A TypeScript 9P client (`workbench/src/wanix/p9-session.ts`,
  `p9-wire.ts`, `p9-path.ts`, `p9.ts`) that is the async analog of
  `crates/wanix-9p-client`. It speaks the same wire to the same
  `P9Server`, framed over WebSocket. This is the only "runtime" the
  browser actually hosts: a wire codec.
- A `WanixP9Handle` plus `WanixHandle` (`fs.js`) that the cockpit uses
  for filesystem-shaped operations against the attached namespace. These
  are the type-safe surface the rest of the cockpit code calls.
- (Slice 9) A `WanixPeer` facade that wraps `P9Session` against a
  remote serve endpoint and exposes `WanixP9Handle`-shaped operations
  against `/n/<id>`. From the cockpit's perspective, a remote peer is
  just another session.
- The VS Code shell, the sidebar, the inspector, the mesh panel: the
  cockpit itself. This is the operator-facing surface; it lives in the
  browser because that is where the user is.
- Session tokens for trusted-loopback attach. These are local to the
  browser, never persisted to the mesh, and refused for non-loopback
  attempts.

**What the browser mounts from peers.**

- The peer's exported namespace at `/n/<peer-id>`. This is the import
  leg of Plan 9's `import`/`export`, realized as a `RemoteFs` bound
  through `Namespace::bind`. Resolving `/n/<peer-id>/...` walks the
  remote peer's filesystem through the mesh.
- The peer's service devices: `/n/<peer-id>/#kv`, `/n/<peer-id>/#agent`,
  `/n/<peer-id>/#cas`, `/n/<peer-id>/#plumb`. Every device is a
  `FileSystem`, so it imports for free through the same client. The
  cockpit's system-view shows each remote device under the `/n/<peer-id>`
  subtree, with the same allocator-vs-control-vs-stream classification.
- Bulk blobs from the peer's content store. The control plane references
  a blob by `ContentHash`; the bytes move on the iroh-blobs data plane,
  BLAKE3-verified end to end, without crawling through the 9P `msize`
  window. Capsules, frozen worlds, and large fixtures all ride this path.
  The cockpit never sees the data-plane transfer; it just sees a hash
  resolve to bytes.
- Pub/sub events on `#plumb` topics shared across peers. The browser
  subscribes to a topic by reading `#plumb/<topic>/recv`; the gossip
  plane fans events out. From the cockpit's perspective, a cross-peer
  event is a read from a remote file. The "subscription" is the open fid.

**What the browser explicitly does not host yet.**

- Full ed25519 identity. The trusted-loopback first cut uses a
  session-token bootstrap from the local serve; persisted ed25519 plus
  iroh QUIC in the browser is the next cycle.
- Public auth. Non-loopback browser peers are blocked. The mesh panel
  shows this policy honestly: peer-attach is loopback-only until a real
  auth story exists.
- Remote `#cpu` initiation without an explicit "I understand this is
  local-trust" prompt. `#cpu` export to arbitrary node IDs stays gated
  until public auth lands.
- v86 emulation. The cockpit-TS spike's direct-v86 bridge is retired.
  The browser is not a runtime tier; it is a peer that mounts a runtime
  tier hosted somewhere else.

The asymmetry matters. The browser *hosts the operator UI*; the browser
*mounts everything else from peers*. There is no "browser-side runtime"
in the sense of v86 emulation or a parallel WASM tier — the browser is a
9P client plus a VS Code shell plus a session-token bootstrap. The
"runtime" is whatever peer the cockpit is attached to. That peer runs
the qjs tasks, the wasm tasks, the agents, the `#cpu` jobs. The cockpit
just files-and-folders its way through them.

This is the reason the merge is tractable: the cockpit does not need to
be re-architected for the mesh. It already speaks `FileSystem`. The
mesh extends `FileSystem` reach to other peers. The cockpit just sees
more files.

It also explains why "the browser is a peer" is not a metaphor. The
browser walks the same `Tattach`/`Twalk`/`Topen`/`Tread`/`Twrite`
sequence as the Rust client; it presents an authenticated identity
(even if loopback-token is a placeholder for ed25519); the server
treats it the same; the grant table covers it; the mesh-panel UI is
just the browser reading the grant device that covers itself. The
operator UI is built on the same boundaries the runtime enforces. There
is no "admin port" or "console mode."

## 5. What the integrated system looks like operationally

The merged story, end to end:

1. A user runs `wanix serve --bundle cockpit $WORKSPACE` (or, for
   backward compatibility, `--bundle workbench-fs9p`). The serve process
   starts the cpu runtime: `#task`, `#term`, `#agent`, `#kv`, `#pipe`,
   `#plumb`, `#cas`, `#cpu` are bound under the local namespace. The
   process generates or loads `~/.wanix/node.key` and stands up the iroh
   `Endpoint` on the three ALPNs. It prints the local `NodeID` and
   listens on the configured WebSocket port for browser attach.

2. The user opens the cockpit URL in a browser. The page loads
   `workbench/dist/web/extension.js` (no 80 MB VS Code zip download from
   origin/rust; the cockpit bundle ships the shell). VS Code activates,
   `bridge.ts` registers `wanix:` as a filesystem provider, `system-view`
   registers the sidebar, `service-inspector` registers `wanix-inspect:`,
   `mesh-panel` registers the mesh view.

3. The cockpit attaches to the local serve over WebSocket-framed 9P,
   presenting a session token. The server treats the cockpit as a
   trusted-loopback peer, grants the local-trust root, and the cockpit's
   `WanixP9Handle` begins resolving paths against the served namespace.
   The same `Tattach` code path that would run for a remote peer runs
   for the cockpit; the only difference is what the policy grants.

4. The sidebar populates from typed discovery JSON (`/.well-known/wanix.json`).
   Categories: drivers (qjs, wasm), tasks (live `#task/<id>` entries),
   terminals (live `#term/<id>` entries), agents (live `#agent/<id>`
   sessions), kv (keys under `#kv`), pipes (active `#pipe` channels),
   plumb (known topics), cas (recent ingest results, presence probes only,
   never enumerated), cpu (local-trust gate state), mesh (identity, peers,
   grants, mounts), activity (recent events), reports (self-check, tool
   contract).

5. The user inspects `#task/<id>`: `wanix-inspect:` opens a read-only view
   of the allocator (the `new` file is unsafe-link, not auto-read), the
   `ctl` file (control-only), the `info` metadata, the `fd/1` and `fd/2`
   streams. Same for `#term/<id>`, `#agent/<id>`, `#pipe/<id>`. The
   classifier is the one from origin/rust; the device coverage is wider.

6. The user runs `wanix.runQjsStarter`. The cockpit reads `#task/new/qjs`
   to allocate a session, writes `cmd`/`env`/`dir` fields, writes `start`
   to `ctl`. The task appears in the sidebar within one poll cycle. The
   inspector shows its fds streaming. The task exits, status is recorded,
   `#task/<id>` clunks. No `globalThis.Wanix` is involved at any layer.

7. The user runs `wanix.runDuetDemo`. Producer task ingests bytes via
   `#cas/ingest`, publishes the resulting hash via `#plumb/duet/send`;
   transform task subscribes via `#plumb/duet/recv`, reads the blob via
   `#cas/<hash>`, ingests the result; verify task reads the final hash
   and compares. All three tasks appear in `#task`; the `#plumb/duet`
   events appear in the activity log; the final hash matches the
   verifier's expected value. The shared-file dance from origin/rust is
   gone; the content-addressed exchange is the demo.

8. The user runs `wanix.runAgentRepair`. Cockpit opens `#agent/new`,
   writes the broken-program fault context to `prompt`, streams events
   from `events` into the activity log, awaits a `reply`, applies the
   patch, re-runs the task. The local fake `AgentEngine` (in
   `crates/wanix-agent`) drives the demo without a remote LLM. The
   broken program exits 0 on the re-run; the cockpit records the repair
   report.

9. The user starts a second serve process on another port (or another
   host). Both processes have mesh enabled. The cockpit's mesh panel
   shows the local `NodeID`, an empty peer list, an empty grant list.
   The user runs `wanix.mesh.dial peer:<other-id>`. The QUIC handshake
   succeeds; the peer appears in the verified-peer list. The user has
   not granted anything, so attach refuses. The user writes a grant
   (subtree prefix, rights). The user runs `wanix.mesh.mount peer:<id> /`.
   The peer's root binds at `/n/<id>` in the local namespace. The cockpit
   sidebar now shows the remote peer's namespace under `/n/<id>` — its
   `#kv`, its `#agent`, its `#cas`, everything that imports as a
   `FileSystem`. Plan 9's `import`, on the real internet, via the
   cockpit.

10. The user runs a `wanix.runDuetDemo` variant where the transform task
    runs on the remote peer. Producer ingests locally; the cockpit
    invokes `#cpu` against the granted peer to run the transform with
    the peer's local fast namespace; verify reads the final hash
    locally. Plan 9's `cpu(1)`, in the cockpit, with the result bytes
    flowing through `#cas` on the data plane and the control flow
    happening through `#plumb` on the gossip plane.

11. The user runs `wanix.runCockpitSelfCheck`. Each of the seven devices
    has a passing probe. `#cpu` enforcement is verified (it refused
    export to a non-loopback synthetic peer). The drivers are present.
    The mesh panel reports a verified peer and an active mount. Report
    is green, archived to `/.wanix/cockpit-check.{json,md}`.

12. The user opens the activity log. Every demo step shows up as a
    `#plumb` event: task started, blob ingested, hash published,
    transform ran, hash verified, peer dialed, grant written, mount
    bound. The activity log is not a side channel; it is a read of
    `#plumb`. The audit trail for the entire session is a file. The
    cockpit can archive it via `#cas/ingest` and reference the result by
    hash, making the session itself a citable object.

The shape is: the cockpit is the operator UI, the runtime is the cpu
substrate, and the mesh extends both transparently. Every cockpit
operation is a file read or write; every device is a `FileSystem`;
every remote thing is a path under `/n/<peer-id>`. No side channels,
no special operator privileges, no parallel control protocol.

It is worth being concrete about what the user *does not* have to do.
They do not have to learn a new tool to inspect a device; the inspector
they used for `#task` works for `#agent`. They do not have to learn a
new tool to reach a peer; the sidebar tree shows `/n/<id>` next to the
local namespace. They do not have to learn a new tool to audit; the
activity log is just `#plumb`. They do not have to learn a new tool to
move bytes; `#cas/ingest` and `#cas/<hash>` are the data plane,
regardless of which peer holds the blob. The integration's whole point
is that one set of primitives — file, device, peer-mounted-as-path —
covers the operator's full job.

## 6. Where to go next

This doc is the unified story. The build sequence and the use-case
catalog live elsewhere.

- **Build sequence**: `docs/integration/plan.md` carries the ten slices
  that take the cpu branch from "stub bridge plus 9P-over-WebSocket
  console" to "cockpit served as the default bundle, mesh panel live,
  every demo re-targeted, browser-as-peer over WebSocket working,
  retired pieces explicitly killed." Slice 1 (bridge plus system-view
  skeleton) is the foundation; Slice 3 (mesh panel) is the externally
  visible payoff of the cpu branch; Slice 9 (browser-as-peer over
  WebSocket) draws the trusted-loopback seam; Slice 10 documents the
  retirements so half-ported demos do not rot in the tree. Read that
  doc when you need to know *what to build next*.
- **Use cases**: `docs/recipes/` is the planned home for end-to-end
  scenarios that exercise the integrated system from the operator's
  point of view. Each recipe is a small story: open the cockpit, do a
  thing, see a result. The duet, the agent repair, the cpu-to-peer
  build, the capsule ticket, the multi-agent handoff over `#plumb` —
  each one is a recipe. Read those when you need to know *what the
  system feels like to use*.
- **Boundary records**: `docs/adrs/0001` through `0005` carry the
  durable architecture decisions the integration must respect: the
  Rust-native Wasmtime runtime boundary (`0001`), the QuickJS/WASI
  task runtime boundary (`0002`), the terminal device and shell
  lifecycle (`0003`), the 9P protocol and server contract (`0004`),
  and the serve and client handoffs (`0005`). The integration extends
  `0005` (or adds `0006`) to record the cockpit-as-operator-UI on the
  mesh boundary; Slice 10 of the plan decides which.
- **Mesh substrate detail**: `docs/mesh-blueprint.md` is the verified
  architecture blueprint for the cpu spike. When the integration
  pushes on a substrate primitive (the 9P client's seekability
  contract, the sync to async bridge, the grant table semantics, the
  CAS data plane), that doc is the source of truth for what the
  substrate guarantees and where the trust boundaries fall.
- **The missing-half framing**: `docs/mesh-the-missing-half-of-9p.md`
  is the conceptual backbone for the cpu spike. It argues that the
  mesh primitives are not a new system layered on top of 9P; they are
  the *client* half of 9P that the original Wanix never built. When
  the integration story asks "why is this a file?" that doc is the
  answer.

The integrated system is not finished. The cpu spike's mesh primitives
are real but the cockpit has not yet been rebuilt on top of them; the
cockpit-TS spike's demos are real but they target a runtime the cpu
branch has retired. The merge is a sequence of file moves, demo
re-pointings, and one honest extension of the cockpit's trust model.
The story is one machine: cockpit on top, mesh underneath, identity
through the middle, every device a file, every peer a path.

## 7. The one-paragraph version

Two spikes, one machine. `origin/rust` built a VS Code cockpit on top
of a single-trust Go-served runtime and proved that an operator UI can
ride a file-shaped device model. `cpu` built a mesh-native Rust runtime
with ed25519 identity, capability grants, a 9P client, and six new
file-shaped devices (`#agent`, `#kv`, `#pipe`, `#plumb`, `#cas`,
`#cpu`), and proved that every Plan 9 mesh primitive collapses to a
`FileSystem`. The merge is to port the cockpit onto the cpu runtime,
re-point every demo at the new devices, retire the Go bridge and the
v86 MessagePort path, and add a mesh panel that surfaces identity,
peers, grants, and mounts as first-class operator categories. The one
real conflict is trust: cockpit-TS had none, mesh-Rust has a
default-deny grant table. The resolution is that the cockpit becomes a
peer, the grant table is itself a file the cockpit reads and writes,
and trusted-loopback attach uses a session-token bootstrap until full
browser-side ed25519 lands. The result is one system where the
operator opens the cockpit, the cockpit walks `#agent`/`#kv`/`#mesh`
over 9P, and agents run on whichever peer the grant table permits. The
build sequence is in `docs/integration/plan.md`; the use cases are in
`docs/recipes/`.
