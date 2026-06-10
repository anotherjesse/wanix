# AGENTS.md

## Rust Wanix Port

North star: build a Rust-native Wanix core that runs outside Chrome, with
Wasmtime as the execution substrate and QuickJS/WASI as the first serious task
runtime. Browser support becomes a frontend or deployment option, not the
runtime foundation. The same core now also reaches across machines: a Plan 9
style mesh imports remote namespaces as local files, so devices, agents, and
compute compose across nodes through the one `FileSystem` contract. Between two
Wanix nodes that contract rides a native FileSystem-over-iroh wire
(`wanix-mesh-wire`); 9P stays the foreign-edge gateway (Linux/v86/QEMU,
external 9P tools, the cockpit).

## Current Big Targets

The baseline qjs, terminal, 9P, direct-v86, and native QEMU paths exist, and on
top of them the mesh/agent layer is built out: service devices (`#pipe`, `#kv`,
`#plumb`, `#cas`, `#agent`), the native FileSystem-over-iroh mesh wire
(`wanix-mesh-wire`, the default Wanix↔Wanix path) and the 9P client
(`wanix-9p-client`) that still mounts foreign namespaces, both over iroh QUIC
with ed25519 identity and default-deny capability binds, the `#cpu` exec plane,
the job protocol (ADR 0009) with ToolFS served per-resource over the mesh
(`wanix tool serve` + the wanix-sh `tool` builtin), `wanix capsule` world
snapshots, and a browser cockpit (VS Code workbench extension) that operates
all of it over direct 9P.

Highest-leverage next work: deepen interactive shells and terminal lifecycle,
broaden Linux/v86/editor 9P compatibility, finish QEMU/v86 boot workflows,
harden the mesh trust boundary (per-principal namespaces, grant lifecycle), and
keep growing the cockpit's coverage of the mesh devices.

## Crate Shape

- `wanix-fs`: filesystem traits, metadata, errors, path rules, in-memory
  fixtures, explicit host-directory-backed filesystems for native demos, and
  `LineBuffer` (the shared bounded, lossy, blocking subscription buffer behind
  `#plumb` recv, AppFS streams, and job `events`).
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
  coupling (command-style subset; `fd_read` blocks on a not-ready device fd
  via bounded-backoff readiness parking per ADR 0010 tier 2, and `poll_oneoff`
  supports exactly `fd_read` readiness subscriptions — everything else stays
  deliberately unsupported).
- `wanix-module-cache`: shared, audited compiled-artifact cache (fd-based
  trust-boundary verification, atomic write, owner-private dir resolver) used by
  both `wanix-qjs-engine` and `wanix-wasm`.
- `wanix-wasm`: compiled-`wasm32-wasi` task driver (`WasmTaskDriver`) and
  free-standing `WasiRunner`, the second WASI task runtime alongside `wanix-qjs`.
  Also ships the installable command package (`COMMANDS`/`command_bin()` over
  `fixtures/commands/<name>.wasm`, e.g. `jaq`).
- `wanix-sh`: the Wanix-native shell — `brush-parser` for bash syntax plus a
  Wanix executor (parse → lower → execute against one `NamespaceOps` seam),
  compiled to `wasm32-wasip1` and run as an ordinary `#task`. Pure of deps except
  `brush-parser`; the guest binary lives at `crates/wanix-wasm/fixtures/shell-src`.

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
- `wanix-mesh-wire`: the native FileSystem-over-iroh wire — a transport-agnostic,
  async-free codec for the `FileSystem`/`File` op-set over the existing sync
  `Duplex` boundary (`postcard` + a 4-byte-LE length-prefixed frame; the QUIC
  stream is the transaction, no tags/`msize`). One stream per call (one-shot
  ops) or per open file (stateful streaming). Carries a typed `WireFsError`
  mirror of `FsError` (lossless, incl. `InvalidPath(String)`/`Other(String)`).
  Exposes `serve_one` (sync server dispatch), `NativeFs`/`NativeFile` (the sync
  client `FileSystem`), and a `StreamFactory` seam the mesh implements. The
  native analog of `wanix-9p` + `wanix-9p-client`. Depends only on `wanix-fs` +
  `wanix-vfs` + `serde` + `postcard` — no iroh/tokio/irpc.
- `wanix-9p-client`: 9P client backed by Wanix filesystem contracts — the
  import half of Plan 9, and the foreign edge. `RemoteFs` mounts a remote 9P
  export as a local `FileSystem`; shares the synchronous 9P core with `wanix-9p`.
- `wanix-cpu`: Plan 9 cpu over the mesh — a `#cpu` acceptor that runs a task
  against a reverse-exported caller namespace, plus the sync, transport-agnostic
  CPU control protocol.
- `wanix-agent`: `#agent` device fronting an LLM session as Wanix files
  (`new`, `prompt`, `events`, `pending`, `ctl`, `reply`, `status`). Backed by a
  codex app-server engine on the local-trust CLI path and a deterministic
  `FakeEngine` on the served path; approvals are files.
- `wanix-job`: the pure job-protocol vocabulary (ADR 0009) — `JobState` with
  the legal-transition relation, the nine-kind `ErrorKind` taxonomy,
  `JobStatus`/`JobResult` report shapes, and the shared `wanix.resource` v0
  spec envelope. serde only; no I/O, no wanix-fs.
- `wanix-jobfs`: the shared job-protocol machinery every job device reuses —
  `JobCore` (job table, lifecycle/TTL expiry, quotas, output caps, the
  finalize timeout backstop), `JobPrincipal`/`JobClock`/`JobLimits`/
  `JobLifecycle`, the `JobRunner` seam with its per-run `RunContext` (job id,
  deadline, abort flag, `events` progress sink), and the job-directory `File`
  impls. Adopters own only their spec shape and path layout.
- `wanix-tool`: ToolFS v0, the first job-protocol device — one host-approved
  operation family as files (`spec.json`, `new`, `jobs/<id>/{in, params.json,
  ctl, out, err, status, result.json, events}`) over `wanix-jobfs`.
  `ToolService` owns the spec surface (and rejects non-`private` visibility:
  v0 implements private job views only); `open_view(principal)` binds a
  principal-scoped `ToolFs`, so identity comes from the attach seam, never a
  payload field. Runners are in-process v0 (deterministic fakes plus
  `ModelRunner` over a `ModelEngine` seam); the process runner is a later
  crate over the `wanix-jobfs` seam.
- `wanix-appfs`: AppFS v0, the file2chan adapter (`docs/appfs.md`) — a
  `FileSystem` whose discrete ops on guest-declared paths become newline-JSON
  request events to a guest app over the sync `AppSender`/`AppReceiver` seam
  (one request in flight under the channel lock; a host pump thread owns guest
  output, fanning `publish` lines out on arrival and latching the channel down
  on any protocol violation, after which ops fail `Unreachable`). Declared
  stream files are host-owned never-EOF subscriptions on the bounded
  drop-oldest `LineBuffer` discipline; `open_view(principal)` binds a
  transport-verified principal (the ToolFS pattern) and `who` lists principals
  holding open stream subscriptions. The CLI wires the channel to a resident
  guest task's pipe ends; the crate itself has no task or engine coupling.
- `wanix-mesh`: the network edge and the only async crate. Binds one
  `iroh::Endpoint` per node from the `wanix-id` secret key and exports/imports a
  namespace over QUIC on two ALPNs: the native wire `WANIX_FS_ALPN`
  (`b"wanix/fs/1"`, the default Wanix↔Wanix path — `dial_native` returns a
  `NativeFs`) and `WANIX_9P_ALPN` for foreign/legacy 9P. It composes
  `wanix-mesh-wire`'s sync codec onto iroh by wrapping each bidi stream in the
  existing `BlockingDuplex` and running `serve_one` on the blocking pool; the
  verified `remote_id()` binds a per-connection principal-scoped `FileSystem`
  view via `wanix-id`'s `AttachPolicy` (default-deny). Reuses the synchronous
  cores unchanged; composes `wanix-cas`/`-cpu`/`-plumb`/`-kv`/`-agent`/`-term`
  across nodes. Because every open file rides its own stream, the old
  `StreamingImportFs` dedicated-stream machinery is retired on the native path.

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

# job protocol: vocabulary + shared machinery + first device
wanix-job   -> serde   (vocabulary only; NO I/O, NO wanix-fs)
wanix-jobfs -> wanix-fs + wanix-job + serde   (job table, lifecycle, runner seam, job-dir files)
wanix-tool  -> wanix-fs + wanix-job + wanix-jobfs

# guest-defined resources: the file2chan adapter
wanix-appfs -> wanix-fs + serde/serde_json + base64   (NO task/engine deps; guest channel is a sync trait seam)

# native mesh wire (transport-agnostic, async-free) + 9P import half + mesh
wanix-mesh-wire -> wanix-fs + wanix-vfs + serde + postcard   (NO iroh/tokio/irpc)
wanix-9p-client -> wanix-fs + wanix-protocol + wanix-9p + wanix-kv
wanix-cpu       -> wanix-9p + wanix-9p-client + wanix-fs + wanix-task + wanix-vfs + wanix-protocol
wanix-mesh      -> wanix-mesh-wire + wanix-9p + wanix-9p-client + wanix-cas + wanix-cpu
                   + wanix-plumb + wanix-kv + wanix-agent + wanix-id + wanix-task + wanix-term
                   + wanix-vfs + iroh/tokio

wanix-cli  -> runtime crates for orchestration
```

`wanix-wasm` now depends on `wanix-task` (it is a task driver, not just a bare
runner). No upward dependencies: `wanix-task` must not depend on `wanix-wasi`,
`wanix-qjs`, or `wanix-wasm`. Keep core filesystem and namespace crates free of
Wasmtime. `wanix-mesh` is the single async/iroh edge: keep tokio and iroh out
of every other crate, including the service devices, the synchronous 9P core,
and `wanix-mesh-wire` (the native wire codec is async-free and speaks only the
sync `Duplex` boundary; `wanix-mesh` is the only crate that binds it to QUIC).

## Current Capability Map

Keep this section concise. Detailed walkthrough output belongs in
`rust-walkthrough.md`; implementation history belongs in commit messages and
tests.

- `wanix-rust qjs main.js`: JavaScript runs outside Chrome as a Wanix `qjs`
  task with live Wanix-backed WASI, namespace access, stdio/fds, env/cwd/cmd,
  observable exit status, and `#task` service files. Tier-2 blocking reads
  (ADR 0010) cover the qjs layer too: `fd_read` on a stdio device fd parks the
  host thread until bytes or EOF (`wanix_wasi::wait`, the one home shared with
  `wanix-wasi-host`), so a resident qjs stdin loop is a working task shape;
  dynamic guest-opened fds stay nonblocking for drain-until-0 device loops.
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
  It is a command-style WASI subset (blocking `fd_read` and `fd_read`-only
  `poll_oneoff` readiness; the rest stays deliberately unsupported).
- `wanix-sh` (`shell.wasm -c "<line>"`): the Wanix-native shell, run as an
  ordinary `.wasm` task. `brush-parser` syntax → a flat `Plan` (and-or lists →
  pipelines → stages, honest `Unsupported` for anything outside the subset) →
  an executor over one `NamespaceOps` seam. Supported: `;` sequences,
  `|` pipelines (concurrent stages per ADR 0010 tier 2: every external stage
  runs on its own host thread via a detached `#task` start and is waited
  through `#task/<id>/wait`, wired through the bounded blocking `#pipe`, EOF
  via `task-exit-closes-fds`),
  `&&`/`||` short-circuit, `< > >>` redirects, `$VAR`/`${VAR}`/`$?` expansion at
  execution time, the `echo`/`cat`/`pwd`/`env`/`true`/`false`/`:`/`exit` pipeable
  builtins plus the pipeable `tool PATH [PARAMS_JSON]` one-shot job-protocol
  client (drives a mounted ToolFS through its visible files, `docs/toolfs.md`)
  and the `cd`/`export`/`unset` special builtins (single-stage), and
  external command launch resolved from a `bin` dir (so `jaq` is just a command —
  `echo '[1,2,3]' | jaq 'map(.+1)'` works) with exported env propagated to
  children. cwd is shell-local (children run at root). Without `-c` the shell
  is an interactive REPL: a `render_prompt` prompt (`$PS1`, `[code]` prefix on
  non-zero `$?`) over blocking byte-wise stdin reads, with guest-owned cooked-
  line editing per ADR 0003 (echo, backspace, Ctrl-C line cancel, Ctrl-D exit) —
  proven end-to-end through `#term` by
  `shell_repl_serves_a_terminal_session_over_blocking_reads`. Tab completion,
  history, and arrow keys are deliberately not built yet. The native-terminal
  CLI entry is `wanix-rust sh` (`-c LINE` or interactive, with `--env`/`--cwd`/
  repeatable `--mount-mesh IROH=/path`). See
  [docs/site/content/concepts/wanix-sh.md](docs/site/content/concepts/wanix-sh.md)
  and `shell-command-resolution.md`.
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
- `wanix-rust p9-stdio` and `serve`: the Rust 9P server exports Wanix
  filesystems over a process pipe (`p9-stdio`, the QEMU-v86 console bridge) and,
  through `serve`, over a binary 9P WebSocket (`/.well-known/export9p`) and raw
  9P over TCP (`serve --p9 ADDR`). One per-connection 9P session core
  (`P9Server::serve_duplex`/`serve_stream`) backs every door; the websocket is a
  `WebSocketDuplex` framing adapter over it, not a second server. Raw-9P export
  is a serve mode (carrying `--peer`/`--grant` capability binds), so the
  retired `p9-listen` "export a dir" use is `serve --root DIR --p9 ADDR`, and
  `mount-write tcp://HOST:PORT ...` mutates a live serve namespace. The
  standalone `p9-listen` and `p9-ws` subcommands are retired (ADR 0006). Binary
  protocol traffic stays separate from diagnostics.
- Serve 9P trust boundary (ADR 0006): `--wanix-services` binds the
  `#task`/`#agent` exec devices (remote code execution) and is refused whenever
  either 9P door (websocket or raw `--p9`) is bound to a non-loopback address,
  mirroring the mesh exec-device rule. The raw `--p9` door is capability-gated
  via `--peer HEX`/`--grant ANAME:PREFIX:RIGHTS` (default-deny `AttachPolicy`).
  `--addr` is deprecated in favor of `--listen`.
- `serve --wanix-services`: exports a Wanix namespace containing the served
  root plus the service devices `#task`, `#term`, `#pipe`, `#kv`, `#plumb`,
  `#cas`, and `#agent` (the inspectable set is the single
  `roots::INSPECTABLE_SERVICE_DEVICES` source, advertised in discovery as
  `services.devices`). Direct 9P clients can allocate/start `noop`/`qjs`/`wasm`
  tasks, attach terminals, and read/write every device through service files.
  WASI guests resolve any `#name` device path from the namespace root, so a
  `qjs`/`wasm` task can open e.g. `#kv/<key>` regardless of its cwd.
- `serve --bind NAME=DIR|NAME=iroh://PEER` (the WebDoor): one generic
  HTTP→namespace gateway on the serve door. Each NAME is one composed
  namespace (static dirs + dialed mesh mounts unioned at the root) served at
  `http://NAME.localhost:PORT` by Host-header routing, so a webapp and the
  room it talks to share one origin (no CORS); bare localhost serves an index
  of bound names. Sized files send whole; zero-length device files stream as
  chunked transfer (SSE `data:` lines under `Accept: text/event-stream`, so a
  browser `EventSource` follows a never-EOF `stream` live); POST/PUT writes;
  dirs serve `index.html` or a JSON listing; `FsError` maps honestly
  (`Unreachable`→503+`Retry-After`). Loopback-only in v0 (ADR 0006-style
  refusal). v0 principal honesty: the gateway dials with ITS dialer key, so a
  mounted room sees one principal for all web users (per-user web identity =
  delegation certs / gateway principals, docs/appfs.md). Demo:
  `examples/chatroom/web` + recipe 09 and
  `docs/site/content/learn/publish-apps-via-web-door.md`; proofs in
  `crates/wanix-cli/src/serve/webdoor/tests.rs`.
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
- The mesh (Plan 9 import realized): between two Wanix nodes `wanix-mesh` carries
  the **native FileSystem-over-iroh wire** (`wanix-mesh-wire`) — `dial_native`
  imports a peer's namespace as a `NativeFs` bound at `/n/<peer>`, one
  `postcard`-framed QUIC bidi stream per call and one per open file (no tags, no
  `msize`), with `FsError` crossing as a typed `WireFsError`. A node binds an
  `iroh::Endpoint` from its persisted ed25519 identity (`wanix-id`) and exports
  its namespace under `WANIX_FS_ALPN` (the native wire) alongside
  `WANIX_9P_ALPN` (the foreign/legacy 9P edge; `wanix-9p-client::RemoteFs` is
  the import half there and the `tcp://` mount path). Attach is capability-gated
  (default-deny grants), bound once per connection from the verified
  `remote_id()` to a principal-scoped `FileSystem` view via `AttachPolicy` — the
  identity is taken from the transport, never carried on the wire. Because every
  device is a plain `FileSystem`, `#kv`/`#cas`/`#plumb`/`#agent` import across
  the mesh for free (`/n/A/#kv/...`); because every open file rides its own
  stream, never-EOF reads need no dedicated-stream machinery
  (`StreamingImportFs` is retired on the native path). Proofs:
  `crates/wanix-mesh/tests/mesh_native*.rs`.
- `#cpu` exec plane: a `wanix-cpu` acceptor runs a task against the caller's
  reverse-exported namespace, so a node can run compute on a peer that operates
  on the caller's files — Plan 9 cpu(1) over the mesh.
- `#agent` device: an LLM session as files (`new`/`prompt`/`events`/`pending`/
  `ctl`/`reply`/`status`), with approvals as files. The CLI `wanix agent` path
  uses the codex app-server engine against a confined Wanix world; the served
  `#agent` uses a deterministic `FakeEngine` (real codex is local-trust only).
  Agents delegate to agents via `#agent/<id>/reply`, and `POST /agent` exposes
  the agent as a network service.
- Job protocol + ToolFS (ADR 0009): `wanix-job` is the shared
  calling-convention vocabulary and `wanix-tool` is ToolFS v0, the first
  device that speaks it — a call is a retained job directory (`new` allocates an opaque id,
  `ctl run` seals input and invokes the runner, validation failures become
  retained failed jobs with taxonomy `result.json`, foreign job ids read as
  `NotFound`, lifecycle is lazy TTL expiry through an injected clock).
  Principal scoping rides `open_view(principal)`, runners are in-process v0
  (fakes plus the `ModelEngine` seam — a model device is just ToolService with
  a model runner). Served over the mesh by `wanix-rust tool serve`. Proofs:
  `crates/wanix-tool/src/tests.rs` pins the docs/toolfs.md validation matrix.
- Composed resource flow (volumes + tools + shell over the mesh, ADR 0007 +
  ADR 0009): `wanix-rust volume create/serve` exports each `~/.wanix/volumes/
  <name>` dir as its own native-wire endpoint (one ticket per resource, persisted
  per-resource ed25519 identity), `wanix-rust tool serve --tool upper|sha256|
  model` does the same for built-in ToolFS devices (every connection bound to a
  private per-principal `jobs/` view from the verified `remote_id()`) and
  `tool serve --config tools.toml` serves *real host programs* through the
  same protocol (the `ProcRunner` fixed-policy inversion: operator-fixed
  absolute command/argv with `{input}`/`{output}` tempfile mapping, no shell,
  empty child env, private per-job temp cwd, real timeout/abort kills with
  retained taxonomy; config names shadow built-ins — walkthrough: recipe 08),
  and
  `wanix-rust sh -c '...' --mount-mesh TICKET=/path ...` composes them: `cat <
  /vol/notes/hello.txt | tool /n/upper > /vol/notes/HELLO.txt` reads one peer,
  runs a job on another, and writes back to the first. The dialer identity
  persists at `~/.wanix/dialer.key`, so one user is one durable principal
  across invocations and remounts (retained jobs and `jobs/` privacy survive a
  remount; distinct keys stay disjoint). `--mount-mesh` is shared task plumbing
  (`crates/wanix-cli/src/mesh/mounts.rs`) carried by `qjs-shell`, `wasm`, and
  `sh`. Walkthrough: `docs/site/content/learn/compose-volumes-and-tools.md` and
  recipe 06 (tested transcript).
- Guest-defined AppResources (`docs/appfs.md`; the ADR 0007 chatroom worked
  example, implemented): `wanix-rust app serve --app DIR --state DIR --addr
  IP:PORT` runs a manifest-declared qjs guest (`app.wanix.json`, the shared
  `"wanix.resource":"v0"` envelope) behind the `wanix-appfs` file2chan adapter
  and exports the resulting `FileSystem` over one native mesh endpoint — one
  ticket names one running app, identity persisted at
  `~/.wanix/app-identities/<name>.key`. The v0 guest is honestly tier-2, not
  tier-1 turns: a detached resident qjs task in a blocking-stdin request loop
  (the qjs-layer blocking `fd_read` above), one discrete op in flight at a
  time as newline-JSON stamped with the verified connection principal
  (`AppAttachPolicy`), with a host pump thread owning guest output. Declared
  stream files are host-owned never-EOF subscriptions (bounded lossy
  `LineBuffer` fan-out fed by guest publishes; `mount-cat PATH --follow` is
  the streaming CLI consumer) with `who` presence from the
  open-subscription registry; durable state is the explicit `--state` mount,
  so the bundled `examples/chatroom` app survives guest restart with history
  intact; a dead guest fails ops `Unreachable` while blocked stream readers
  are released with EOF (auto-restart is opt-in: `--restart on-failure`
  re-runs an exited guest with capped backoff behind the same ticket; serve
  death is abrupt — only guest-exit-while-serve-lives gets the clean EOF
  release). Proofs:
  `crates/wanix-appfs/src/tests.rs`, `crates/wanix-cli/src/app/serve/tests.rs`;
  walkthrough: recipe 07 + `docs/site/content/learn/build-a-chatroom.md`.
- `wanix-rust capsule`: freezes a Wanix world into a portable, CAS-backed
  `.wcap` (via `wanix-cas`) that can be loaded elsewhere; live mesh peers and
  ephemeral handles are not portable.

The biggest missing pieces remain interactive shell/session depth, broader
Linux/v86/editor 9P compatibility, QEMU/v86 boot workflows, and hardening the
mesh trust boundary (grant lifecycle, and per-fid per-principal namespaces on
the 9P plane — the native wire already binds a per-connection principal-scoped
view from the verified `remote_id()`). The serve 9P websocket handles one frame
at a time per connection, so a blocking read (e.g. `#plumb/<topic>/recv`) cannot
be interleaved with a write on the same connection — live pub/sub needs a second
connection or concurrent frame handling. (The native wire does not have this
constraint: every open file rides its own QUIC stream.) Ethernet/vnet and
public/multi-user auth remain explicitly unimplemented trust-boundary work.

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

- [ADR 0000](docs/adrs/0000-agent-native-environment.md) **(MANIFESTO /
  PROPOSED)**: the agent-native philosophy record, written first-person by the
  resident agent — durable state outside model context, authority-as-namespace,
  one fabric and one calling convention, audit by construction, an operable
  task lifecycle, transport identity, old-world-as-mounts; the native agent
  loop built from Wanix substrates instead of ported harnesses; and the four
  agent-first questions (discover/invoke/audit/grant) every new device
  contract must answer.
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
- [ADR 0004](docs/adrs/0004-rust-9p-protocol-and-server-contract.md): the
  `FileSystem`/`NamespaceOps` trait is the source of truth; the native
  FileSystem-over-iroh wire (`wanix-mesh-wire`) is the Wanix↔Wanix mesh encoding
  and 9P (`wanix-protocol`/`wanix-9p`) is the foreign-edge gateway codec. Carries
  open questions on per-attach (`aname`) scoping, in-band device EOF, and the
  read-chunk clamp (ADR 0004 §Open questions).
- [ADR 0005](docs/adrs/0005-serve-and-client-handoffs.md): Rust `serve`,
  browser filesystem/workbench, direct-v86, rootfs prep, and native QEMU share
  explicit discovery and handoff contracts instead of becoming runtime
  foundations.
- [ADR 0006](docs/adrs/0006-serve-9p-transport-and-trust-boundary.md): one
  per-connection 9P session core (`serve_duplex`/`serve_stream`) backs every
  serve 9P door; the websocket is a `WebSocketDuplex` framing adapter and raw 9P
  is a serve mode (`--p9 ADDR`), so `p9-ws`/`p9-listen` are retired; the serve
  9P edge refuses `--wanix-services` off-loopback and capability-gates raw `--p9`
  via `--peer`/`--grant`.
- [ADR 0007](docs/adrs/0007-resources-catalogs-and-pairing.md) **(DRAFT /
  PROPOSED)**: the resource catalog / volume server / host-wrapper device /
  recipe story — a humane mesh front door where you address resources by name
  and compose namespaces from a catalog — plus the layered trust model (Layer 0
  iroh identity → Layer 1 naming → Layer 2/3 authorization). A discussion draft,
  to be split into accepted ADRs once its contracts stabilize.
- [ADR 0008](docs/adrs/0008-live-mesh-resource-liveness.md): live `iroh://`
  resource liveness and retry semantics for agent/shell mounts — hard vs. soft
  mounts, operation deadlines, stale handle behavior, no replay of in-flight
  mutations, stale route hints, and the backoff/status state machine target.
- [ADR 0009](docs/adrs/0009-job-protocol.md) **(PROPOSED)**: the job protocol —
  reified calls as the workspace calling convention. Slow/effectful/abortable
  work is invoked as a job directory (`new`, `jobs/<id>/{in,params.json,ctl,
  out,err,status,result.json}`); the job id is the idempotency key (preserving
  ADR 0008's no-replay rule while giving callers a resolution path); one shared
  error taxonomy; ToolFS is the first implementation.
- [ADR 0010](docs/adrs/0010-task-tiers-turns-and-snapshots.md) **(PROPOSED)**:
  the task concurrency and operability model — tier 1 turn-based resident
  tasks (single-actor, host-owned handles/pumping, snapshot/kill/fork/migrate
  at empty-stack turn boundaries), tier 2 POSIX-ish command tasks (per-task
  host threads, bounded blocking pipes, killable via epoch interruption, not
  migratable), tier 3 whole-OS guests (v86/QEMU) as the escape hatch; no async
  or threads in guests; `#task/<id>/ctl kill`.

The mesh/agent layer (the native FileSystem-over-iroh wire, ed25519 identity +
capability binds, the `#agent` device, and the `#kv`/`#pipe`/`#plumb`/`#cas`/
`#cpu` service contracts) is now split across ADRs only where contracts have
stabilized: the native wire's contract is owned by
[ADR 0004](docs/adrs/0004-rust-9p-protocol-and-server-contract.md) (FileSystem
contract, native mesh wire, and the 9P edge gateway) with the op-by-op design in
[docs/design/native-mesh-wire.md](docs/design/native-mesh-wire.md), and live
resource liveness is owned by
[ADR 0008](docs/adrs/0008-live-mesh-resource-liveness.md). The rest of the layer
is captured in [docs/mesh-blueprint.md](docs/mesh-blueprint.md) and
[docs/mesh-the-missing-half-of-9p.md](docs/mesh-the-missing-half-of-9p.md) (both
written against the earlier 9P-over-iroh build; see their update notes), and the
cockpit↔mesh integration in [docs/integration/](docs/integration/). Promote the
durable boundaries (mesh identity/trust; agent-as-device; the service device
contracts) into consecutive ADRs when those contracts stabilize.

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

- Native mesh wire open questions ([ADR 0004](docs/adrs/0004-rust-9p-protocol-and-server-contract.md)
  §Open questions): (a) **read-chunk clamp** — bind it to the advertised
  `iounit_hint` (`min(max, MAX_CHUNK_LEN, iounit_hint)`); a cheap latent-coupling
  fix, do it before any open path advertises a smaller per-file hint. (b)
  **`FileReply::Eof`** — decide drop-as-YAGNI (the simple default; no server path
  emits it) vs keep-reserved for a device needing in-band close distinct from a
  stream drop. (c) **per-attach `aname` scoping** — the native wire resolves
  `AttachPolicy` at the root attach name only (`dial_native` discards `aname`), so
  9P-style `ANAME`-keyed sub-scoping is unreachable; settle it *with* the
  ADR 0006/0007 authorization layer (thread `aname` 1:1 vs a native
  scope-selection shape), not before. (a) and (b) are small cleanup-cycle items.
- Job protocol / ToolFS remainder ([docs/toolfs.md](docs/toolfs.md) §Build
  Slices): the fixed-command process runner shipped (`tool serve --config
  tools.toml`, `crates/wanix-cli/src/tool/process.rs`); still ahead are
  catalog integration (one entry per served tool) and the agent adapter.
  `--mount-mesh` still needs threading to `qjs`/`qjs-term` (the keepalive
  home, `mesh::mounts`, already exists). `#agent`/`#cpu` convergence on the
  job grammar waits until those devices are next touched (ADR 0009 §Adopters).
- AppResource v0 remainder ([docs/appfs.md](docs/appfs.md) §Build Slices /
  §Status): the served guest is a tier-2 resident qjs loop, not the tier-1
  turn model with a host handle table (ADR 0010) — move it when turns land.
  Wire v0.2 shipped the op deadline (`DEFAULT_OP_DEADLINE`), host-stamped
  `at_ms`, ranged reads, and the hello handshake; `--restart on-failure`
  shipped the guest restart policy; the WebDoor (`serve --bind`) shipped the
  generic HTTP gateway. Still ahead: CAS-pinned app manifests (`main` is read
  from `--app` by path; provenance is a doc note only), per-user gateway
  principals / a guest `fetch` handler (`wanix/http/1`) so web users stop
  collapsing into the gateway's one principal, and an off-loopback gateway
  auth story (today a non-loopback `--listen` with `--bind` is refused at
  startup).
- Task-kill gaps (documented on `EpochInterrupter` and `Task::kill`): (a) a
  task parked inside a blocking *host* read (e.g. a quiet stdin) only dies on
  its next return to guest code — make the host `Backoff` park loops
  kill-aware; (b) the qjs driver has no per-task interrupt seam (all qjs tasks
  share one engine), so kill on a running qjs task only sets the observable
  `kill_requested` flag; (c) no process groups: while a foreground external
  runs, the shell's Ctrl-C watcher drains stdin and preserves non-Ctrl-C bytes
  as type-ahead for the next prompt instead of delivering them to a child that
  reads inherited stdin.
- CLI UX remainder: the hands-on new-user audit behind commit bc05331 landed
  only its top S/M findings (lean usage errors, per-subcommand `--help`,
  ADR 0008 unreachable text, split peer-id parse diagnostics, copy-pasteable
  serve announcements); its larger findings — anything needing a CLI
  behavior-contract change — were deliberately deferred and are not recorded
  as tickets, so re-derive them by running the binaries as a new user before
  the next UX pass.
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
- 9P session/namespace seam: on the **9P plane**, `handle_attach` decodes
  `uname`/`aname` and discards them in the no-policy path; every fid resolves
  through one shared `P9Server.root`. (The **native wire** does the non-no-op
  version already: it binds the verified `remote_id()` once per connection to a
  principal-scoped `FileSystem` view via `AttachPolicy`, never a client-claimed
  `uname` — see `crates/wanix-mesh/tests/mesh_native_identity.rs`.) On 9P, a
  per-fid per-principal root for multiple concurrent attaches only lands cleanly
  alongside per-fid root storage and a first consumer (HTTP-worker or per-user
  namespaces) — do not add a no-op seam, since a discarded provider result is a
  dead abstraction.
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
