# AGENTS.md

## Rust Wanix Port

North star: build a Rust-native Wanix core that runs outside Chrome, with
Wasmtime as the execution substrate and QuickJS/WASI as the first serious task
runtime. Browser support becomes a frontend or deployment option, not the
runtime foundation. The same core now also reaches across machines: a Plan 9
style mesh imports remote namespaces as local files, so devices, agents, and
compute compose across nodes through the one 9P contract.

## Current Big Targets

The baseline qjs, terminal, 9P, direct-v86, and native QEMU paths exist, and on
top of them the mesh/agent layer is built out: service devices (`#pipe`, `#kv`,
`#plumb`, `#cas`, `#agent`), the 9P client (`wanix-9p-client`) that mounts
remote namespaces, 9P over iroh QUIC with ed25519 identity and default-deny
capability binds, the `#cpu` exec plane, `wanix capsule` world snapshots, and a
browser cockpit (VS Code workbench extension) that operates all of it over
direct 9P.

Highest-leverage next work: deepen interactive shells and terminal lifecycle,
broaden Linux/v86/editor 9P compatibility, finish QEMU/v86 boot workflows,
harden the mesh trust boundary (per-principal namespaces, grant lifecycle), and
keep growing the cockpit's coverage of the mesh devices.

## Crate Shape

- `wanix-fs`: filesystem traits, metadata, errors, path rules, in-memory
  fixtures, and explicit host-directory-backed filesystems for native demos.
- `wanix-protocol`: dependency-free wire protocol helpers, currently centered
  on 9P frame splitting, tag extraction, version negotiation, and
  server-facing 9P2000.L/Google compatibility codecs.
- `wanix-9p`: 9P server adapters backed by Wanix filesystems, including fid
  state, metadata/mutation mapping, compatibility probes, and synchronous
  byte-stream transport helpers.
- `wanix-vfs`: Plan 9-style namespace binding and resolution.
- `wanix-task`: task model, `#task`, fd table, and driver registry.
- `wanix-term`: terminal device filesystem for `#term/new`, `<id>/ctl`,
  `<id>/data`, `<id>/program`, and `<id>/winch`.
- `wanix-wasi`: custom WASI Preview 1 imports backed by Wanix namespaces and
  task fds supplied by adapter configuration.
- `wanix-qjs-engine`: Wasmtime-hosted QuickJS/WASI engine mechanics relocated
  from the `rust-wasi-quickjs` prototype, keeping the existing Rust import path
  while using the workspace-local QuickJS WASM fixture. Crate-local engine
  invariants live in `crates/wanix-qjs-engine/docs/architecture.md`; do not add
  one Wanix ADR per engine helper.
- `wanix-qjs`: QuickJS/WASI task driver that adapts the engine crate to Wanix
  task semantics.
- `wanix-wasi-host`: standalone Wasmtime WASI Preview 1 linker for the compiled
  wasm runner, generic over a `WasiHost` backing and carrying no engine or task
  coupling (command-style subset; `poll_oneoff` is `NOSYS`).
- `wanix-module-cache`: shared, audited compiled-artifact cache (fd-based
  trust-boundary verification, atomic write, owner-private dir resolver) used by
  both `wanix-qjs-engine` and `wanix-wasm`.
- `wanix-wasm`: compiled-`wasm32-wasi` task driver (`WasmTaskDriver`) and
  free-standing `WasiRunner`, the second WASI task runtime alongside `wanix-qjs`.

Service-device and mesh crates (the distributed layer; each device is a plain
`FileSystem`, so it imports across the mesh for free):

- `wanix-kv`: `#kv` key/value service filesystem; `#kv/<key>` reads/writes a
  value, the device is the durable state primitive for app/agent state.
- `wanix-pipe`: `#pipe` in-memory byte channels (`#pipe/new` allocates,
  `<id>/data` is the unidirectional read/write end, EOF on last writer drop).
- `wanix-plumb`: `#plumb` plumber bus; `#plumb/<topic>/send` publishes a
  newline-JSON envelope and `#plumb/<topic>/recv` drains envelopes received
  since open. `LocalPlumbPort` is single-node; the mesh swaps in a gossip port.
- `wanix-cas`: content-addressed store (venti): `ContentStore` trait,
  owner-private `LocalCasStore`, the `#cas` device (`<hash>` read, `ingest`
  write-then-read-hash, `have/<hash>` probe), and CAS-backed `.wcap` capsules.
- `wanix-id`: node identity (persisted ed25519 `NodeIdentity`) and default-deny
  capability grants (`AttachPolicy`, `GrantTable`) for the mesh trust boundary.
- `wanix-9p-client`: 9P client backed by Wanix filesystem contracts — the
  import half of Plan 9. `RemoteFs` mounts a remote 9P export as a local
  `FileSystem`; shares the synchronous 9P core with `wanix-9p`.
- `wanix-cpu`: Plan 9 cpu over the mesh — a `#cpu` acceptor that runs a task
  against a reverse-exported caller namespace, plus the sync, transport-agnostic
  CPU control protocol.
- `wanix-agent`: `#agent` device fronting an LLM session as Wanix files
  (`new`, `prompt`, `events`, `pending`, `ctl`, `reply`, `status`). Backed by a
  codex app-server engine on the local-trust CLI path and a deterministic
  `FakeEngine` on the served path; approvals are files.
- `wanix-mesh`: the network edge and the only async crate. Binds one
  `iroh::Endpoint` per node from the `wanix-id` secret key, exports a namespace
  as 9P over QUIC (ALPN), and dials peers to import their namespaces as
  `RemoteFs`. Reuses the synchronous 9P core; composes `wanix-cas`/`-cpu`/
  `-plumb`/`-kv`/`-agent`/`-term` across nodes.

- `wanix-cli`: native CLI and demo runner (includes `serve`, the mesh
  subcommands, and `capsule`).
- Future protocol work beyond the current 9P stack: CBOR/RPC, HTTPFS, and R2FS
  protocol pieces when those integrations need them.

The browser cockpit lives in `workbench/` (a VS Code / Code OSS web extension,
TypeScript): it drives the served namespace and all of the devices above over
direct 9P (`workbench/src/wanix/p9.ts`), and is the human operator surface for
the Rust runtime. It is a frontend, not a runtime foundation.

## Dependency Direction

Keep lower-level crates independent from runtime engines:

```text
wanix-fs
  -> wanix-vfs
  -> wanix-task

wanix-protocol
wanix-9p -> wanix-fs + wanix-protocol
wanix-term -> wanix-fs + wanix-task + wanix-vfs
wanix-wasi -> wanix-fs + wanix-task + wanix-vfs
wanix-wasi-host -> wanix-fs + wanix-wasi + Wasmtime
wanix-module-cache -> Wasmtime
wanix-qjs-engine -> Wasmtime + QuickJS WASM fixture + wanix-module-cache
wanix-qjs  -> wanix-fs + wanix-vfs + wanix-task + wanix-wasi + wanix-qjs-engine + wanix-module-cache
wanix-wasm -> wanix-fs + wanix-vfs + wanix-task + wanix-wasi + wanix-wasi-host + wanix-module-cache

# service devices: plain FileSystems over wanix-fs
wanix-kv | wanix-pipe | wanix-plumb | wanix-agent -> wanix-fs (+ wanix-vfs)
wanix-cas -> wanix-fs + wanix-module-cache
wanix-id  -> wanix-fs + wanix-vfs

# 9P import half + mesh
wanix-9p-client -> wanix-fs + wanix-protocol + wanix-9p + wanix-kv
wanix-cpu       -> wanix-9p + wanix-9p-client + wanix-fs + wanix-task + wanix-vfs + wanix-protocol
wanix-mesh      -> wanix-9p + wanix-9p-client + wanix-cas + wanix-cpu + wanix-plumb
                   + wanix-kv + wanix-agent + wanix-id + wanix-task + wanix-term + wanix-vfs + iroh/tokio

wanix-cli  -> runtime crates for orchestration
```

`wanix-wasm` now depends on `wanix-task` (it is a task driver, not just a bare
runner). No upward dependencies: `wanix-task` must not depend on `wanix-wasi`,
`wanix-qjs`, or `wanix-wasm`. Keep core filesystem and namespace crates free of
Wasmtime. `wanix-mesh` is the single async/iroh edge: keep tokio and iroh out
of every other crate, including the service devices and the synchronous 9P core
that the mesh reuses.

## Current Capability Map

Keep this section concise. Detailed walkthrough output belongs in
`rust-walkthrough.md`; implementation history belongs in commit messages and
tests.

- `wanix-rust qjs main.js`: JavaScript runs outside Chrome as a Wanix `qjs`
  task with live Wanix-backed WASI, namespace access, stdio/fds, env/cwd/cmd,
  observable exit status, and `#task` service files.
- `wanix-rust wasm FILE.wasm [args]`: the compiled-`wasm32-wasi` task runtime
  (the `wanix-wasm` `WasiRunner` path) and second WASI task driver alongside
  `qjs`. `.wasm` is a first-class Wanix task kind: a `.wasm` cmd auto-starts via
  `#task/new` through `WasmTaskDriver` (`check` matches `.wasm`, `start` reads
  the module from the task's namespace, builds a live WASI config from the
  task's namespace/cwd/env/argv and fds 0/1/2, runs `_start`, and records the
  guest exit through `Task::set_exit`), exposes `#task` service files, and has
  observable task-level exit. It shares one `Namespace`/VFS with `qjs` and
  follows the same fd-mirroring contract (ADR 0002) through the shared
  `wanix-wasi` task config path. The CLI `wasm` subcommand and the
  `shared_vfs_differential` test plus the `compute_bench`/`shared_vfs` examples
  run a Rust wasm task and a qjs task against one `MemFs` and observe identical
  filesystem state, proving the compiled-vs-interpreted tier on one substrate.
  It is a command-style WASI subset (no `poll_oneoff` readiness).
- `wanix-rust qjs-term main.js` and `wanix-rust qjs-shell`: terminal-backed
  `qjs` tasks bind fd 0/1/2 through `#term/<id>/program`; native cooked/raw
  shell modes and served shell sessions use the same `#term` device contract,
  with a small filesystem command set (`cd`, `ls`, `cat`, `write`, `mkdir`,
  `rm`, `rmdir`, `mv`, `cp`, `ln -s`, `readlink`, `stat`, and `lstat`),
  `env`/`setenv`/
  `unsetenv` task-environment commands, `ps` task-table inspection, and
  synchronous child `qjs` task launches with inherited env, direct terminal
  stdio including buffered foreground stdin handoff, namespace stdio
  redirection, queued resize tracking from `#term/<id>/winch`, and observable
  child exit status for demos. Served shell sessions release their owned
  terminal resource when the session closes, and discovery advertises the
  qjs-shell cwd query, resize formats, exit frame, and session lifecycle
  contract for browser/editor clients.
- `wanix-rust p9-stdio`, `p9-listen`, `p9-ws`, and `serve`: the Rust 9P server
  exports Wanix filesystems over process, TCP, WebSocket, and HTTP composition
  layers, with binary protocol traffic kept separate from diagnostics.
- `serve --wanix-services`: exports a Wanix namespace containing the served
  root plus the service devices `#task`, `#term`, `#pipe`, `#kv`, `#plumb`,
  `#cas`, and `#agent` (the inspectable set is the single
  `roots::INSPECTABLE_SERVICE_DEVICES` source, advertised in discovery as
  `services.devices`). Direct 9P clients can allocate/start `noop`/`qjs`/`wasm`
  tasks, attach terminals, and read/write every device through service files.
  WASI guests resolve any `#name` device path from the namespace root, so a
  `qjs`/`wasm` task can open e.g. `#kv/<key>` regardless of its cwd.
- `serve --bundle fs9p`: browser filesystem client over direct 9P.
- `serve --bundle workbench-fs9p`: the browser cockpit — a Code OSS/workbench
  launch path where the bootstrap passes the discovered direct-9P route into the
  extension, negotiates Google.2 `walkgetattr` when available, browses and
  mutates `wanix:/` over direct 9P, preserves symlink metadata for editor file
  types, and opens qjs-backed terminal sessions when services are enabled.
  Static workbench/vscode-web assets are served from the repo `workbench/` tree
  (not the served root) so a disposable root still boots. The Wanix activity-bar
  view is the operator surface: it inspects the service devices over 9P
  (`wanix-inspect:`), runs the agent repair demo through `#agent` (session +
  approval), runs the qjs→wasm→qjs duet on one shared FS, serves HTTP apps via
  `/.wanix/app/<name>` with `#kv`-backed state, and self-checks the device set.
  Direct terminal disposal writes `close` through `#term/<id>/ctl`.
- `serve --bundle direct-v86`: browser v86 handoff over Rust serve discovery,
  direct 9P, boot-asset hints, hvc0 console bridging with Ctrl-C/Ctrl-D byte
  forwarding and a scriptable hvc0 send hook, direct-v86 9P `msize` discovery
  and query override, and autostart-friendly launch hooks. The generated page
  reports rootfs handoff status including executable `/bin/init` readiness and,
  for trusted loopback clients, exposes `window.wanixRootfsHandoff` plus copyable
  QEMU/direct-v86 serve commands.
- `wanix-rust rootfs --archive FILE.tgz --out DIR`: prepares a guest root,
  rejects unsafe archive paths, validates VM boot markers including executable
  `/bin/init`, and emits shell or `wanix-rootfs.v1` JSON handoffs for QEMU and
  direct-v86 without owning rootfs build or VM lifecycle.
- `/.well-known/rootfs.json`: when loopback clients access `serve` on a
  prepared guest root, exposes the same `wanix-rootfs.v1` handoff for trusted
  local browser/editor/VM launchers.
- `wanix-rust qemu --root DIR`: validates the same guest-root shape, including
  an executable default `/bin/init` marker unless `--cmdline` owns init policy,
  and emits a shell or `wanix-qemu-virtio9p.v1` JSON handoff with discovered or
  explicit initrd support plus an explicit 9P `msize` boot knob; `--exec` is an
  explicit foreground launch, not a Wanix VM supervisor.
- The mesh (Plan 9 import realized): `wanix-9p-client::RemoteFs` mounts a remote
  9P export as a local `FileSystem`, and `wanix-mesh` carries 9P over iroh QUIC.
  A node binds an `iroh::Endpoint` from its persisted ed25519 identity
  (`wanix-id`), exports its namespace under the Wanix ALPN, and dials peers to
  import theirs at `/n/<peer>`. Attach is capability-gated (default-deny grants),
  and because every device is a plain `FileSystem`, `#kv`/`#cas`/`#plumb`/
  `#agent` import across the mesh for free (`/n/A/#kv/...`).
- `#cpu` exec plane: a `wanix-cpu` acceptor runs a task against the caller's
  reverse-exported namespace, so a node can run compute on a peer that operates
  on the caller's files — Plan 9 cpu(1) over the mesh.
- `#agent` device: an LLM session as files (`new`/`prompt`/`events`/`pending`/
  `ctl`/`reply`/`status`), with approvals as files. The CLI `wanix agent` path
  uses the codex app-server engine against a confined Wanix world; the served
  `#agent` uses a deterministic `FakeEngine` (real codex is local-trust only).
  Agents delegate to agents via `#agent/<id>/reply`, and `POST /agent` exposes
  the agent as a network service.
- `wanix-rust capsule`: freezes a Wanix world into a portable, CAS-backed
  `.wcap` (via `wanix-cas`) that can be loaded elsewhere; live mesh peers and
  ephemeral handles are not portable.

The biggest missing pieces remain interactive shell/session depth, broader
Linux/v86/editor 9P compatibility, QEMU/v86 boot workflows, and hardening the
mesh trust boundary (per-principal namespaces, grant lifecycle). The serve 9P
websocket handles one frame at a time per connection, so a blocking read (e.g.
`#plumb/<topic>/recv`) cannot be interleaved with a write on the same
connection — live pub/sub needs a second connection or concurrent frame
handling. Ethernet/vnet and public/multi-user auth remain explicitly
unimplemented trust-boundary work.

## Code Quality Guardrails

Modules should stay under 250-350 non-test lines. A module over 250 lines needs
a clear reason to keep growing; a module over 350 lines should be split before
new feature work lands there. Existing over-limit production modules are tracked
in `tools/module-line-baseline.txt`; they may shrink, but they must not grow
without an explicit cleanup decision. Do not hold a namespace or filesystem lock
while calling into another filesystem.

Use explicit Rust types for public contracts. Avoid public raw `i32` flags,
file descriptors, rights, or modes where newtypes/builders make the trust
boundary clearer.

Required checks before a cycle commit:

```sh
just check
```

## ADR Index

Keep this index to active architecture, API, format, trust-boundary, and
workflow contracts. The active set is intentionally small and consecutive.
Milestone proofs and progress-journal records belong in tests, examples,
current-state docs, and commit messages instead of active ADRs.

- [ADR 0001](docs/adrs/0001-rust-native-wasmtime-runtime.md): Rust Wanix is the
  host/microkernel runtime, Wasmtime is the execution substrate, Go is a
  migration oracle, and workspace crates stay layered around Wanix-owned
  contracts.
- [ADR 0002](docs/adrs/0002-quickjs-wasi-task-runtime.md): the WASI task runtime
  boundary (QuickJS `.js` and compiled `.wasm`); Wanix owns task identity, live
  WASI semantics, fd/service state, snapshots reattachment, fixture boundaries,
  and bounded guest execution policy for any WASI runtime, while the runtime
  crate owns engine mechanics.
- [ADR 0003](docs/adrs/0003-terminal-device-and-shell-lifecycle.md):
  `wanix-term` and terminal-backed native, served, editor, and VM sessions
  share one terminal device and shell lifecycle contract.
- [ADR 0004](docs/adrs/0004-rust-9p-protocol-and-server-contract.md):
  `wanix-protocol` and `wanix-9p` own the Rust 9P frame, codec, fid, metadata,
  mutation, and compatibility contract across transports.
- [ADR 0005](docs/adrs/0005-serve-and-client-handoffs.md): Rust `serve`,
  browser filesystem/workbench, direct-v86, rootfs prep, and native QEMU share
  explicit discovery and handoff contracts instead of becoming runtime
  foundations.

The mesh/agent layer (9P-over-QUIC transport, ed25519 identity + capability
binds, the `#agent` device, and the `#kv`/`#pipe`/`#plumb`/`#cas`/`#cpu`
service contracts) does not yet have ADRs; its design is captured in
[docs/mesh-blueprint.md](docs/mesh-blueprint.md) and
[docs/mesh-the-missing-half-of-9p.md](docs/mesh-the-missing-half-of-9p.md), and
the cockpit↔mesh integration in [docs/integration/](docs/integration/). Promote
the durable boundaries (mesh transport + identity; agent-as-device; the service
device contracts) into consecutive ADRs when those contracts stabilize.

## Runtime Guardrails

These are current guardrails for keeping retired bridge ideas out of new work:

- Guest JavaScript should use `qjs:std`, `qjs:os`, `scriptArgs`, stdio, env, and
  service files instead of `globalThis.Wanix` helpers.
- Wanix runtime paths should use live Wanix-backed WASI providers; engine
  read-only virtual files are fixture support only.
- Raw shell input should flow through `#term`; guest shells own echo, simple
  editing, newline handling, Ctrl-C line cancellation, Ctrl-D exit, and command
  dispatch. Browser/editor/VM terminal clients should forward control bytes;
  they should not invent task cancellation semantics around them.
- Rust-served workbench paths should pass discovered direct-9P routes into the
  extension; the MessagePort/CBOR bridge is browser-embedded compatibility only.

## ADR Workflow

Treat ADRs like code. Add or update one only for durable architecture, API,
format, trust-boundary, or workflow decisions. When touching a topic, review the
related ADRs at the same time and consolidate, delete, or clearly retire records
that no longer describe the current direction. ADRs should stay terse: record
the boundary and consequences, then point implementation status to tests,
examples, walkthrough docs, and commit messages.

The root ADR index above is the active Wanix decision set: runtime boundary,
QuickJS/WASI task boundary, terminals, 9P, and serve/client handoffs. Prefer
revising one of those records before adding a new one. Crate-local invariants
belong in crate docs, but nested crates should not grow separate Wanix ADR
series. Keep active ADRs focused on current contracts.

Milestone proofs, fixture rebuild notes, per-syscall or per-operation coverage,
CLI/demo slices, and smoke-test progress belong in tests, examples,
current-state docs, and commit messages. If an ADR draft reads like a good
commit message, keep it as the commit message instead. Routine cargo command
lists live in the code-quality guardrails above unless the workflow itself
changes.

## Cycle Rules

Prefer the highest-leverage externally visible capability or demo outcome.
Use cleanup only when it unblocks that outcome, protects a trust boundary,
preserves compatibility, or fixes a major review finding. Commit each completed
cycle before starting the next one. Every roughly five feature commits, run a
review/cleanup pass over the changes since the last cleanup pass before adding
more feature work.

## Queued Follow-ups

- Module-line health: `just module-lines` is green against the 350-line hard
  limit, but three modules sit above the 250-line warn limit and should be split
  before they grow — `wanix-agent/src/codex.rs` (~307), `wanix-agent/src/
  exec_server.rs` (~283), and `wanix-cli/src/serve/http/app.rs` (~273, restored
  from the cockpit work). Keep running `just module-lines` during cleanup.
- Cockpit follow-ups: `v86-shared-demo` is still a no-op stub in
  `workbench/src/web/extension.ts` (marked `// STUB:`); port it next. `#plumb`
  live receive in the self-check probes the publish path only because a blocking
  recv would deadlock the single-frame-at-a-time serve connection — wiring a
  second 9P connection (or concurrent frame handling) would let it verify
  end-to-end delivery. Consider Slice 8 from `docs/integration/plan.md` (rename
  `--bundle workbench-fs9p` to `--bundle cockpit`, retire the `workbench/code/`
  vscode-web vendor dependency).
- Serve concurrency: `serve/concurrent.rs` spawns a detached worker thread per
  connection with no cap and busy-polls `accept()` on a fixed sleep in unbounded
  mode. A proper fix (connection cap + thread accounting) needs a shutdown signal
  first — there is none anywhere in `serve/` today, so the cap is otherwise
  untestable and unconditional `JoinHandle` retention would leak handles forever.
  Do the shutdown signal + cap together as one cycle, before HTTP workers /
  remote / multi-user.
- Typed discovery/handoff JSON: discovery (`serve/discovery.rs`), rootfs, qemu,
  and direct-v86 handoffs are still hand-built `format!` JSON. The driver-list
  drift is fixed (discovery now derives drivers from the registry), but convert
  the remaining fragments to typed structs with shape-pinning tests as a cleanup.
- 9P session/namespace seam: `handle_attach` decodes `uname`/`aname` and discards
  them; every fid resolves through one shared `P9Server.root`. A per-principal
  `NamespaceProvider` only lands cleanly alongside per-fid root storage and a
  first consumer (HTTP-worker or per-user namespaces) — do not add the trait as a
  no-op seam, since a discarded provider result is a dead abstraction.
- Continue `qjs-shell` interactivity with true resize wakeups independent of
  stdin handling, persistent foreground child-task terminal ownership,
  cancellation, command execution beyond the current built-ins and synchronous
  `qjs` launcher with file stdio redirection, and richer terminal/session
  lifecycle control.
- Decide the auth/WebSocket policy needed for browser v86 and VS Code
  integration, then extend the direct-v86 route into a complete qemu/v86 bundle,
  `/.well-known/ethernet`, vnet, and VS Code routes on the Rust `serve`
  endpoint.
- Add backing contracts for 9P special files or extended attributes only when a
  Linux/v86/editor workflow requires them.
