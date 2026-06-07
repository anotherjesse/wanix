# Cockpit + Mesh Integration Plan

## Direction

The `cpu` branch reshaped Wanix into a mesh-native Rust runtime. Identity
(`wanix-id`), QUIC transport (`wanix-mesh`), a synchronous 9P client
(`wanix-9p-client`), and a family of file-shaped devices — `#agent`, `#kv`,
`#pipe`, `#plumb`, `#cas`, `#cpu`, alongside the existing `#task` and `#term` —
now live in `crates/`. The browser today is reduced to a 9P-over-WebSocket
console (`workbench/src/wanix/p9*.ts`) plus a stub VS Code activation
(`workbench/src/web/extension.ts`, `workbench/src/web/bridge.ts`).

The `origin/rust` branch, in contrast, evolved the workbench into a full
operator cockpit: a `WanixSystemView` tree of drivers/tasks/terminals/agents, a
`WanixServiceInspector` for `#task`/`#term` introspection, demo modules
(`agent-repair-demo`, `duet-demo`, `http-app-demo`, `qjs-starter`,
`wasm-starter`, `v86-shared-demo`), a `cockpit-self-check` runtime probe, an
`agent-tool-contract` publisher, and a 40-command VS Code `extension.ts` that
threads them together.

The integrated direction is: **cockpit is the operator UI for the mesh**.
Concretely:

1. Restore the cockpit shell (`bridge.ts`, `system-view.ts`,
   `service-inspector.ts`, `extension.ts` commands, `workbench-assets.ts`) on
   top of the existing cpu `WanixP9Handle` + `WanixHandle`. These pieces are
   VS Code-coupled and have no Rust equivalent.
2. Re-target every demo and probe at cpu's new devices. `agent-repair-demo`
   becomes a real `#agent` session driver instead of a hand-rolled `qjs`
   template. `duet-demo` and `http-app-demo` keep their pedagogical narrative
   but route output through `#task`, `#cas` (for shared blobs), and `#plumb`
   (for cross-runtime events). `cockpit-self-check` probes the new device list
   (`#agent`, `#kv`, `#pipe`, `#plumb`, `#cas`, `#cpu`) in addition to
   `#task`/`#term`.
3. Add a new cockpit primitive — a **mesh panel** — that surfaces verified
   peers, granted subtrees, and `/n/<node>` mounts. This is the externally
   visible payoff of the cpu branch and has no analog on `origin/rust`.
4. Make the browser a real Wanix peer where it pays off. The TypeScript 9P
   client in `workbench/src/wanix/p9-session.ts` is the async analog of
   `crates/wanix-9p-client`; consolidating around it (Option B in the recon)
   lets the cockpit attach to a remote namespace as a first-class peer over
   WebSocket-framed 9P, without WASM-binding the sync Rust client. Full mesh
   peerhood (ed25519 identity, iroh QUIC in the browser) is deferred behind a
   discovery flag; the first deliverable is **trusted-loopback browser-as-peer
   over WebSocket**, which the existing `p9-ws` listener already accepts.
5. Retire what no longer fits: the Go-served `globalThis.Wanix` bridge,
   `direct-v86` MessagePort/CBOR coupling, the `workbench-fs9p` plain bundle
   path as the primary cockpit entry, and any demo that relied on a runtime
   feature `cpu` does not have. Each retirement is called out in the slices.

The success bar for the merge is: `wanix-rust serve --bundle cockpit
$WORKSPACE` brings up VS Code with a sidebar that shows the cpu device map and
mesh state, lets an operator run qjs/wasm tasks through `#task`, open an
`#agent` session, post and receive on `#plumb`, ingest into `#cas`, and mount a
loopback peer's namespace at `/n/<id>`, with `cockpit-self-check` green across
every device on cpu.

## Slices

### 1. Cockpit bones: bridge + system-view skeleton on cpu's WanixP9Handle

**Goal.** Restore the minimal cockpit shell — `WanixBridge`, `WanixSystemView`,
`WanixServiceInspector` — on top of cpu's existing
`workbench/src/wanix/p9.ts` (`WanixP9Handle`) and `workbench/src/wanix/fs.js`
(`WanixHandle`). No demos yet; no mesh yet. Just: VS Code mounts `wanix:` as a
filesystem provider, the sidebar shows a tree, the inspector handles
`wanix-inspect:`, and `extension.ts` activates cleanly against cpu's 9P
discovery (`/.well-known/wanix.json`). This re-introduces the architectural
seam that everything else lands on.

**Files touched.**

- `workbench/src/web/bridge.ts` (restore full FileSystemProvider; today it is a
  stub on cpu).
- `workbench/src/web/system-view.ts` (port from `origin/rust`, but stripped to
  drivers/tasks/terminals/namespace/activity — no demo-specific categories
  yet).
- `workbench/src/web/service-inspector.ts` (port unchanged; it already aligns
  with cpu's `#task`/`#term` allocator+ctl pattern documented in
  `crates/wanix-task/src/task_files.rs` and
  `crates/wanix-term/src/files.rs`).
- `workbench/src/web/extension.ts` (replace stub with activation that wires
  bridge + system-view + inspector; keep command count low — just
  `wanix.refresh`, `wanix.openSystemJournal`, `wanix.openServiceInspector`).
- `workbench/src/web/workbench-assets.ts` (port; needed by later slices that
  ship `.wasm` fixtures).
- `crates/wanix-cli/src/serve/workbench_fs9p.html` (rename concept to
  `cockpit.html` or keep filename but treat as the cockpit shell — see Slice
  8).

**Success.** `cd workbench && npm run compile-web` produces a clean bundle;
`wanix-rust serve --bundle workbench-fs9p $WORKSPACE` opens VS Code, the
`wanix:` scheme browses cpu's served root, the system-view sidebar lists
drivers (`qjs`, `wasm` from `crates/wanix-qjs`, `crates/wanix-wasm`) and any
live `#task`/`#term` entries discovered via 1-second polling of
`config.ns.task` / `config.ns.term`. `wanix-inspect:` URIs open read-only
introspection of `#task/<id>` and `#term/<id>` with allocator/ctl/stream files
correctly classified per
`/Users/jesse/lw/wanix/crates/wanix-agent/src/files.rs:23-58`
allocator-pattern semantics.

**Depends on.** None.

---

### 2. Device-aware system view: surface #agent, #kv, #pipe, #plumb, #cas, #cpu

**Goal.** Extend `WanixSystemView` to model the new cpu devices as
first-class categories alongside the existing tasks/terminals tree. Each
device gets a sub-tree whose nodes follow the device's structural pattern
(allocator vs. metadata-store vs. pub/sub vs. content-store) documented in the
recon. `service-inspector.ts` is widened to handle the new ctl/stream files
without inviting the user to read an allocator's `new` file (which would
allocate).

**Files touched.**

- `workbench/src/web/system-view.ts` (add `agents`, `kv`, `pipes`, `plumb`,
  `cas`, `cpu`, `mesh` categories; agents list active `#agent/<id>` sessions
  by polling, kv lists keys via `read_dir` of `#kv`, pipes list active
  channels, plumb lists known topics, cas exposes recent `ingest` results +
  presence probes only — never lists blobs).
- `workbench/src/web/service-inspector.ts` (mark `new` files in `#agent`,
  `#pipe`, `#term` as unsafe-link to prevent inadvertent allocation; mark
  `ctl` files as control-only).
- `workbench/src/web/device-discovery.ts` (new module: reads
  `/.well-known/wanix.json` and the cpu serve `--wanix-services` discovery
  output to learn which devices are mounted under which namespace prefixes;
  drives system-view categories).
- `crates/wanix-cli/src/serve/discovery.rs` (extend JSON payload to enumerate
  bound device prefixes — `#agent`, `#kv`, etc. — so the cockpit can render
  them without a hard-coded list. This is the typed-discovery cleanup already
  queued in the AGENTS Queued Follow-ups; do the minimal subset needed by the
  cockpit here).

**Success.** With `wanix-rust serve --wanix-services --bundle workbench-fs9p`,
the sidebar shows seven device categories. Allocating an `#agent` session via
`cat \#agent/new` from a terminal makes a new session appear in the sidebar
within one poll cycle; closing it via `\#agent/<id>/ctl close` removes it.
Writing a key under `#kv` makes it appear under the kv category. Discovery
JSON exposes the device list and the cockpit derives categories from it.

**Depends on.** 1.

---

### 3. Mesh panel: verified peers and /n/<node> mounts

**Goal.** Add the externally visible payoff of the cpu branch: a mesh panel in
the cockpit that shows the local `NodeIdentity` (`crates/wanix-mesh/src/identity.rs`),
verified peer connections via QUIC (`crates/wanix-mesh/src/handler.rs`,
`crates/wanix-mesh/src/dialer.rs`), the `GrantTable` (which peer is granted
which subtree prefix), and the set of `/n/<node>` mounts that the local
namespace currently binds. Operators can issue `mesh.dial` and `mesh.mount`
commands from VS Code.

**Files touched.**

- `workbench/src/web/mesh-panel.ts` (new module: TreeDataProvider for
  `wanix.mesh` view; reads node identity, peer list, grant list, mount list
  via discovery + a small set of service-file probes).
- `workbench/src/web/extension.ts` (register `wanix.mesh.dial`,
  `wanix.mesh.mount`, `wanix.mesh.revokeGrant` commands; thread mesh-panel
  refresh through `bridge.onDidWanixMutation`).
- `crates/wanix-cli/src/serve/discovery.rs` (expose
  `mesh.identity.peer_id`, `mesh.peers[]`, `mesh.grants[]`, `mesh.mounts[]`
  via the same typed payload extended in Slice 2).
- `crates/wanix-mesh/src/lib.rs` (expose a small inspection API — peer
  enumeration, grant snapshot — without breaking the trust boundary; today
  these live as internal state).
- `crates/wanix-cli/src/serve/command.rs` (add `--mesh-listen`, `--mesh-dial`
  flags if not present; the recon shows mesh-on-serve already exists, this
  slice just makes it visible).

**Success.** Two local `wanix-rust serve` processes with mesh enabled show
each other in the cockpit mesh panel within one poll. Running `wanix.mesh.mount
peer:<id> /` from the cockpit binds the peer's root at `/n/<id>` in the local
namespace (visible immediately in the `wanix:` tree). `wanix.mesh.revokeGrant`
removes a grant and the peer's subsequent `Tattach` returns EACCES. Default
remains deny: a peer with no grant cannot mount anything.

**Depends on.** 1, 2.

---

### 4. Demos re-targeted: qjs-starter, wasm-starter, duet over #task + #cas

**Goal.** Port `qjs-starter.ts`, `wasm-starter.ts`, and `duet-demo.ts` from
origin/rust against the cpu drivers (`crates/wanix-qjs`,
`crates/wanix-wasm`) and cpu's `#task`/`#cas`. Template installation stays in
TypeScript (it is VS Code editor UX). Execution flows through `#task/new/qjs`
and `#task/new/wasm` directly. The duet's shared blob handoff (producer →
transform → verify) is rewritten to go through `#cas` ingest, replacing the
origin/rust `/duet/shared/*` plain-file dance with a content-addressed
exchange that demonstrates cpu's CAS device end-to-end.

**Files touched.**

- `workbench/src/web/qjs-starter.ts` (port; replace any `globalThis.Wanix`
  hooks with `#task/new/qjs` allocator reads + `cmd`/`env`/`dir` writes +
  `ctl start`; reference cpu's `crates/wanix-task/src/task_files.rs` field
  semantics).
- `workbench/src/web/wasm-starter.ts` (port; uses `#task/new/wasm`; binary
  fetched via `workbench-assets.ts` and written to scratch path; execution
  via `#task`).
- `workbench/src/web/duet-demo.ts` (port; producer writes blob via
  `#cas/ingest`, publishes hash via `#plumb/duet/send`, transform subscribes
  via `#plumb/duet/recv`, reads blob via `#cas/<hash>`, ingests result;
  verify reads final hash and compares).
- `workbench/media/rust-guest.wasm` (re-add the fixture wasm from
  origin/rust; needed by wasm-starter and duet's transform step).
- `workbench/src/web/extension.ts` (register `wanix.runQjsStarter`,
  `wanix.runWasmStarter`, `wanix.installDuetDemo`, `wanix.runDuetDemo`
  commands; thread output through TaskOutputRecorder pattern from
  origin/rust extension.ts:1584).

**Success.** From the cockpit, `Install duet demo` writes templates;
`Run duet demo` produces a final hash in `#cas` that matches the verifier's
expected value, with `#plumb/duet` events visible in the sidebar timeline. All
three tasks appear and exit cleanly in `#task` with status codes recorded.
Re-running the demo is idempotent (templates not duplicated).

**Depends on.** 1, 2.

---

### 5. Agent repair demo on #agent device

**Goal.** Re-implement `agent-repair-demo.ts` as a real `#agent` session
flow instead of the hand-rolled `repairQjsProgram` deterministic fixer that
origin/rust used. The demo installs a broken qjs program, opens an `#agent`
session via `#agent/new`, writes the fault context to `prompt`, streams
`events` into the cockpit's activity log, awaits a `reply`, applies it,
re-runs the task, and reports success. The fake/local `AgentEngine` in
`crates/wanix-agent/src/router.rs` can drive the demo without a remote LLM;
the same code path works against a `RemoteEngine` wrapping `/n/<peer>/#agent`
once Slice 3 is in place.

**Files touched.**

- `workbench/src/web/agent-repair-demo.ts` (port; replace `repairQjsProgram`
  internal fixer with an `#agent` session: open `#agent/new`, write to
  `prompt`, read from `events`, apply patch, run task; mark the original
  deterministic fixer **retired** with reason: cpu has a real agent device
  and the demo should exercise it).
- `workbench/src/web/agent-tool-contract.ts` (port; update the 8-tool surface
  to reference cpu paths — `readFile`→`wanix:/`, `runTask`→`#task/new/<kind>`,
  `observeTask`→`#task/<id>/fd/{1,2}`, `previewHttpRoute`→drop until Slice 7,
  `writeSystemSnapshot`→`#cas/ingest`).
- `workbench/src/web/extension.ts` (register
  `wanix.installAgentRepair`, `wanix.runAgentRepair`,
  `wanix.openAgentToolContract`).
- `crates/wanix-agent/src/router.rs` (no code change expected; verify the
  fake engine returns a usable patch envelope the demo can apply).

**Success.** From the cockpit, `Run agent repair demo` opens an `#agent`
session visible in the sidebar, the system-view activity log streams events
from `#agent/<id>/events`, the broken program is patched and re-run with exit
0, and a repair report is written. Re-running re-opens a fresh session and
closes the old one via `ctl close`.

**Depends on.** 1, 2, 4.

---

### 6. Cockpit self-check and tool contract against cpu devices

**Goal.** Port `cockpit-self-check.ts` and align `agent-tool-contract.ts` so
the cockpit can prove the runtime is ready on cpu. The self-check probes:
drivers present (`qjs`, `wasm`), `#task` reachable, `#term` reachable,
`#agent` allocator reachable (allocate then close), `#kv` round-trip, `#pipe`
round-trip, `#plumb` send/recv, `#cas` ingest+presence, `#cpu` not exported
publicly (verifies the local-trust gate). Results write to
`/.wanix/cockpit-check.{json,md}`. Tool contract publishes the cpu tool
surface to `/.wanix/agent-tools.{json,md}` for downstream agent backends.

**Files touched.**

- `workbench/src/web/cockpit-self-check.ts` (port; rewrite probe list to cover
  the seven devices plus drivers; reference each probe back to the
  corresponding `crates/wanix-*` device).
- `workbench/src/web/agent-tool-contract.ts` (final pass; document the
  cpu-aware tool surface).
- `workbench/src/web/extension.ts` (register `wanix.runCockpitSelfCheck`;
  invoke on activation behind a `wanix.runSelfCheckOnStart` setting).
- `crates/wanix-cli/src/serve/discovery.rs` (final shape of the typed
  discovery payload; pin with a test in `crates/wanix-cli/src/serve/tests.rs`
  so the cockpit's check does not break silently).

**Success.** `Run cockpit self-check` is green on a default
`wanix-rust serve --wanix-services` run: each of the seven devices has a
passing probe, plus drivers and `#cpu` local-trust enforcement. The self-check
report appears in the sidebar's reports category.

**Depends on.** 1, 2, 3, 4, 5.

---

### 7. HTTP app demo and route orchestration over #task + #plumb

**Goal.** Port the high-leverage `http-app-demo.ts` route orchestrator from
origin/rust. The demo discovers `/apps/*.{js,wasm}`, dispatches each through
`/.wanix/app/<name>`, captures `x-wanix-task-id` / stdout / stderr response
headers, and renders the result. On cpu this route is served by a small new
HTTP route in `wanix-cli`'s serve layer that allocates a `#task`, binds the
request body as stdin, returns stdout, and emits a `#plumb/http` event for
each invocation so the cockpit's activity log can show the request stream
without polling.

**Files touched.**

- `workbench/src/web/http-app-demo.ts` (port; replace go-bridge HTTP route
  with cpu's `/.wanix/app/<name>` route; consume `#plumb/http` for live
  event display).
- `crates/wanix-cli/src/serve/http/routes.rs` (add `/.wanix/app/<name>` route
  that resolves `/apps/<name>.{js,wasm}`, allocates a `#task` via the local
  device, runs it, returns stdout with tracing headers; publishes to
  `#plumb/http` for cockpit observability).
- `crates/wanix-cli/src/serve/discovery.rs` (advertise the http-app route
  template so the cockpit knows where to dispatch).
- `workbench/src/web/extension.ts` (register `wanix.installHttpAppDemo`,
  `wanix.openHttpAppDemo`, `wanix.openHttpAppCatalog`,
  `wanix.previewHttpAppPath`; the catalog/source/preview command split from
  origin/rust survives unchanged).

**Success.** From the cockpit, `Install HTTP app demo` writes
`/apps/hello.js` and `/apps/echo.wasm`. Invoking
`http://127.0.0.1:7654/.wanix/app/hello` returns the qjs handler's response
with valid `x-wanix-task-id` and stdout/stderr trace paths; the cockpit
sidebar's activity log shows the request via `#plumb/http`. WASM handler
behaves identically.

**Depends on.** 1, 2, 4, 6.

---

### 8. Cockpit bundle: rename, default route, retire workbench-fs9p as primary

**Goal.** Make the cockpit the default served bundle. Rename the bundle name
in `wanix-cli` from `workbench-fs9p` to `cockpit` while keeping
`workbench-fs9p` as a backward-compat alias. Update the embedded HTML
(`crates/wanix-cli/src/serve/workbench_fs9p.html`) so its asset paths match
the `workbench/dist/web/` build output rather than the now-deleted
`workbench/code/out/...` VS Code zip layout. Retire any direct-v86
MessagePort/CBOR bridge code from the workbench bootstrap (per the cpu
guardrail in CLAUDE.md that workbench paths should pass discovered direct-9P
routes into the extension).

**Files touched.**

- `crates/wanix-cli/src/serve/command.rs` (accept `--bundle cockpit`; map
  `workbench-fs9p` to the same handler).
- `crates/wanix-cli/src/serve/html.rs` (`bundle_html` resolves both names to
  the cockpit HTML; rename the constant to `COCKPIT_HTML`).
- `crates/wanix-cli/src/serve/workbench_fs9p.html` (rename file to
  `cockpit.html`; rework asset paths to load `workbench/dist/web/extension.js`
  rather than vendored VS Code paths).
- `crates/wanix-cli/src/serve/http/routes.rs` (`bundle_response` matches both
  query values; mark `?bundle=workbench-fs9p` as deprecated in the startup
  banner).
- `crates/wanix-cli/src/serve.rs` (startup banner advertises `cockpit` first).
- `workbench/build.go` (verify esbuild output layout matches the new asset
  paths).

**Success.** `wanix-rust serve --bundle cockpit $WORKSPACE` is the documented
entry point. `?bundle=workbench-fs9p` still works but logs a deprecation
hint. The HTML loads `workbench/dist/web/extension.js` directly without
needing the 80 MB VS Code distribution download from origin/rust's
`make -C workbench build`. All slices 1–7 are exercisable from this single
URL.

**Depends on.** 1.

---

### 9. Browser-as-peer over WebSocket: trusted-loopback first cut

**Goal.** Make the browser a real Wanix peer for the trusted-loopback case.
The TypeScript 9P client (`workbench/src/wanix/p9-session.ts`,
`p9-wire.ts`, `p9-path.ts`, `p9.ts`) is the async analog of
`crates/wanix-9p-client`. Take Option B from the recon: consolidate the
TS client and surface it through a small `WanixPeer` facade that the cockpit
can use to mount a peer's exported namespace at `/n/<id>` from the browser
side. Identity for v1 is the loopback-only bootstrap — the browser presents
a session token issued by the local `serve` process, not a persisted ed25519
keypair. Full ed25519 identity + iroh QUIC in the browser is deferred to a
later cycle; this slice draws the seam.

**Files touched.**

- `workbench/src/wanix/peer.ts` (new module: `WanixPeer` facade; constructs
  a `P9Session` against a remote `serve` endpoint, exposes
  `WanixP9Handle`-shaped operations against `/n/<id>`).
- `workbench/src/web/mesh-panel.ts` (add a "Mount peer from browser" command
  that prompts for ws:// URL + token, calls `WanixPeer.connect`, binds the
  result into the cockpit's filesystem view at `/n/<id>`).
- `crates/wanix-cli/src/p9_ws/connection.rs` (accept a session token query
  parameter on the WebSocket handshake; refuse non-loopback peers without a
  matching grant entry — mirrors the Slice 4 exec-export gate from the mesh
  blueprint).
- `crates/wanix-cli/src/serve/discovery.rs` (advertise the peer-attach
  endpoint and current loopback-only policy so the cockpit can show it
  honestly).
- `docs/integration/plan.md` (this file: append a follow-up note that full
  ed25519 + iroh in the browser is the next cycle).

**Success.** Two `wanix-rust serve` processes on the same host: cockpit on
process A mounts process B's root over WebSocket from the browser, browses
files, runs a `#task` on B, observes the result. Without a token, the
WebSocket attach is refused. Non-loopback browser peers are explicitly
blocked.

**Depends on.** 1, 2, 3, 6, 8.

---

### 10. Retire & document: explicit kill-list of origin/rust pieces that do not survive

**Goal.** Close the merge by explicitly retiring origin/rust pieces that have
no counterpart on cpu, and recording why. This avoids the slow-rot risk of
half-ported demos. Also restores the two design docs the recon flagged as
deleted (sprint plan, browser DX report) into `docs/integration/` as historical
references, since they document the visual design intent of features that
this plan re-targets onto cpu.

**Files touched.**

- `docs/integration/retired.md` (new: lists every origin/rust workbench file
  with its disposition — ported, ported-with-changes, or retired-with-reason).
  Specific retirements expected:
  - `direct-v86` MessagePort/CBOR bridge in workbench bootstrap — replaced
    by cpu's direct-9P route discovery (per CLAUDE.md guardrail).
  - `repairQjsProgram` deterministic fixer in `agent-repair-demo` —
    superseded by `#agent` (Slice 5).
  - Go-served `globalThis.Wanix` helpers — replaced by `qjs:std`, `qjs:os`,
    service files (per CLAUDE.md guardrail).
  - `/duet/shared/*` plain-file handoff — replaced by `#cas` + `#plumb`
    (Slice 4).
  - Any cockpit code that polled Go-side task state — replaced by polling
    `#task` service files (Slices 1, 2).
- `docs/integration/origin-rust-cockpit-sprint.md` (restore the deleted
  `docs/wanix-os-cockpit-month-sprint.md` from origin/rust as a historical
  reference under `docs/integration/`).
- `docs/integration/origin-rust-browser-dx.md` (restore the deleted
  `docs/wanix-workbench-browser-dx.md` similarly; flag images as not
  re-imported, just text).
- `docs/adrs/` (decide whether the integrated cockpit deserves a new ADR
  alongside `0005-serve-and-client-handoffs.md`; per the AGENTS ADR workflow,
  prefer revising 0005 rather than adding 0006 unless the cockpit boundary
  is genuinely a new trust contract).
- `workbench/src/web/` (delete or leave-disabled any half-ported file that
  did not make it through Slices 1–9; do not leave dead modules in tree).

**Success.** `docs/integration/retired.md` covers every TS module from the
recon's enumeration (`agent-repair-demo`, `agent-tool-contract`,
`bridge`, `cockpit-self-check`, `duet-demo`, `extension`,
`http-app-demo`, `qjs-starter`, `service-inspector`, `system-view`,
`v86-shared-demo`, `wasm-starter`, `workbench-assets`) with a one-line
disposition pointing at the slice that handled it or the reason for
retirement. `workbench/src/web/` contains no dead files. ADR 0005 (or a new
0006) reflects the cockpit-as-operator-UI boundary on the mesh.

**Depends on.** 1, 2, 3, 4, 5, 6, 7, 8, 9.

## Open questions

- **Browser identity, full peerhood**: Slice 9 stops at trusted-loopback over
  WebSocket. The mesh blueprint sketches two paths for browsers to become full
  ed25519 peers (localStorage seed vs. grant-from-peer ticket). Picking one is
  out of scope for this merge but should be decided in the cycle following
  Slice 10. The discovery flag in Slice 9 should carry forward.
- **`v86-shared-demo`**: origin/rust used `/shared/` for v86<->browser file
  sharing. On cpu this could become a `#cas` + `#plumb` cross-VM demo, or
  stay a flat 9P-shared directory. The recon flagged v86 demos as still
  useful pedagogy; deciding which device backs it determines whether it
  earns a slice or rolls into Slice 4. Default in this plan: rolled into
  Slice 4's CAS+plumb story unless a v86 boot scenario specifically needs the
  flat-shared pattern.
- **`workbench/code/` VS Code distribution**: origin/rust's `make -C workbench
  build` downloads ~80 MB of VS Code. Slice 8 sidesteps this by loading
  `workbench/dist/web/extension.js` directly, but a real VS Code-style UI
  still needs the host shell. Decide whether the cockpit bundle ships its own
  trimmed host or continues to depend on the upstream `vscode-web` zip.
- **Typed discovery JSON cleanup**: Slices 2, 3, 6, 7 all extend
  `crates/wanix-cli/src/serve/discovery.rs`. The AGENTS Queued Follow-ups
  already calls out converting discovery to typed structs with shape-pinning
  tests. This merge raises the urgency; ideally land the typed cleanup as a
  prep step before Slice 2 rather than accreting `format!()` JSON across the
  slices.
- **`#cpu` exposure in the cockpit mesh panel**: Slice 3 surfaces peer
  identity and grants. `#cpu` (remote task execution) stays local-trust by
  policy. Decide whether the mesh panel should also let an operator
  *initiate* a `#cpu` call against a granted peer, or whether that stays a
  command-line capability until public auth lands. Default: surface the
  primitive but gate it behind a "I understand this is local-trust" prompt.
- **End-to-end Playwright smoke test**: the recon found no e2e tests but
  Playwright is available. A single smoke test that boots
  `wanix-rust serve --bundle cockpit`, opens the page, asserts the seven
  device categories render, and runs the duet demo end-to-end would be the
  highest-leverage protection against the cockpit silently breaking. Likely
  one extra slice between 8 and 9, or rolled into 10.
