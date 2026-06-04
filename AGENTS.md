# AGENTS.md

## Rust Wanix Port

North star: build a Rust-native Wanix core that runs outside Chrome, with
Wasmtime as the execution substrate and QuickJS/WASI as the first serious task
runtime. Browser support becomes a frontend or deployment option, not the
runtime foundation.

## Current Big Targets

The baseline qjs, terminal, 9P, direct-v86, and native QEMU paths now exist.
Highest-leverage next work should make those paths feel like a usable system:
interactive shells and terminal lifecycle, broader Linux/v86/editor 9P
compatibility, QEMU/v86 boot workflows, and serve/workbench/VS Code
integration.

## Crate Shape

- `wanix-fs`: filesystem traits, metadata, errors, path rules, in-memory
  fixtures, and explicit host-directory-backed filesystems for native demos.
- `wanix-protocol`: dependency-free wire protocol helpers, starting with 9P
  frame splitting, tag extraction, version negotiation, and basic
  server-facing 9P2000.L operation codecs.
- `wanix-9p`: 9P server adapters backed by Wanix filesystems, starting with
  an in-process frame handler for attach/walk/open/read/write/clunk and
  directory listing, plus a synchronous byte-stream transport loop.
- `wanix-vfs`: Plan 9-style namespace binding and resolution.
- `wanix-task`: task model, `#task`, fd table, and driver registry.
- `wanix-term`: terminal device filesystem for `#term/new`,
  `<id>/data`, `<id>/program`, and `<id>/winch`.
- `wanix-wasi`: custom WASI Preview 1 imports backed by Wanix namespaces and
  task fds supplied by adapter configuration.
- `wanix-qjs-engine`: Wasmtime-hosted QuickJS/WASI engine mechanics relocated
  from the `rust-wasi-quickjs` prototype, keeping the existing Rust import path
  while using the workspace-local QuickJS WASM fixture.
- `wanix-qjs`: QuickJS/WASI task driver that adapts the engine crate to Wanix
  task semantics.
- `wanix-cli`: native CLI and demo runner.
- Future protocol work beyond the current 9P stack: CBOR/RPC, HTTPFS, and R2FS
  protocol pieces when those integrations need them.

## Dependency Direction

Keep lower-level crates independent from runtime engines:

```text
wanix-fs
  -> wanix-vfs
  -> wanix-task

wanix-protocol
wanix-9p -> wanix-fs + wanix-protocol
wanix-term -> wanix-fs
wanix-wasi -> wanix-fs + wanix-vfs
wanix-qjs-engine -> Wasmtime + QuickJS WASM fixture
wanix-qjs  -> wanix-task + wanix-wasi + wanix-qjs-engine
wanix-cli  -> runtime crates for orchestration
```

No upward dependencies: `wanix-task` must not depend on `wanix-wasi` or
`wanix-qjs`. Keep core filesystem and namespace crates free of Wasmtime.

## Current Vertical Slices

The baseline qjs demo is `wanix-rust qjs main.js`: JavaScript runs outside
Chrome, reads and writes files through a Wanix namespace, prints through
task-backed stdio, exits with an observable status, and can read `#task/self/id`
to prove task context crosses into QuickJS.

The next terminal-facing demo is `wanix-rust qjs-term main.js`: JavaScript still
runs as a Wanix `qjs` task, but fd 0/1/2 are bound through `#term/<id>/program`
and the native binary streams terminal `data` output while the task runtime is
live. The checked-in
`qjs-term-ready-io-demo.js` also proves QuickJS ready-IO handlers can consume
terminal-backed fd 0 in bounded turns. The `--feed-after-eval`,
`--feed-after-eval-file`, and `--feed-after-eval-lines` options prove terminal
input can arrive after JS has registered a read handler while the runtime
remains live, including deterministic line-by-line scripted sessions and
post-eval native stdin line streaming. Terminal fd readiness is queue-aware, so
ready-IO pumps can run while a read handler is armed without firing on empty
terminal input. Post-eval terminal feeds stop when the qjs task requests a
Wanix process exit, which lets the shell demo's `exit` command return without
waiting for native stdin EOF. The user-facing shell demo is now
`wanix-rust qjs-shell`, which runs the bundled QuickJS shell source through the
same terminal-backed Wanix task runtime with line-oriented native input.
`qjs-shell --raw` adds native raw-mode setup and feeds native bytes directly
through the terminal device, letting the bundled QuickJS shell own echo, simple
editing, Ctrl-D, and command dispatch inside the Wanix task.
`qjs-shell` still accepts `--event-loop-ms` for deterministic scripted feeds;
on Unix, native process IO also passes a pollable stdin fd into the shell loop
and pumps bounded QuickJS event-loop work while stdin is idle, so delayed shell
output can surface before the next native input byte. The Unix native shell
also polls the host terminal size through stdout, delivers changed dimensions
through `#term/<id>/winch`, and exposes the latest queued resize through the
bundled shell's `size` command.
`qjs-term --resize-after-eval COLSxROWS` sends a deterministic post-eval
resize event to `#term/<id>/winch` as `columns rows\n`, proving QuickJS tasks
can observe terminal resize broadcasts through live Wanix-backed fd readiness.

The next 9P-facing demo target is wiring the native and browser listeners into
serve/v86 experiments so external clients can browse a Wanix namespace. The
server core already negotiates 9P2000.L and caps Google-extension negotiation
at `9P2000.L.Google.2`, attaches, walks, opens regular files and read-only
directories, reads/writes regular files, lists directories with `Treaddir`,
clunks fids, reports no-follow metadata with `Tgetattr` and Google.2
`Twalkgetattr`, creates
and reads symbolic links with `Tsymlink` and `Treadlink`, and handles core mount
and mutation ops with `Tstatfs`, `Tlcreate`, `Tmkdir`, legacy `Trename` and
`Tremove`, `Trenameat`, and `Tunlinkat`, file size, permission, and timestamp
mutation with `Tsetattr`,
session-virtual uid/gid ownership through `Tsetattr`/`Tgetattr`,
advisory-lock compatibility probes with `Tlock`/`Tgetlock`, synchronous
compatibility probes with `Tflush`, Google.1 `Tflushf`, and `Tfsync`, and
append-open write semantics for `O_APPEND` fids; it also decodes `Tmknod`,
`Tlink`, and xattr
probes as typed compatibility requests and returns intentional unsupported
errors until Wanix grows backing contracts, while `Tauth` returns explicit
`ENOSYS` because Rust Wanix does not require a separate 9P auth phase yet. It
serves encoded request/response frames through a synchronous stream loop.
`wanix-rust p9-stdio --root DIR` exposes that server over process
stdin/stdout, with stdout reserved for binary 9P responses and stderr reserved
for human-readable CLI or transport errors. `wanix-rust p9-listen --root DIR
--addr 127.0.0.1:5640` exposes the same `LocalFs` export over a native TCP
listener. `wanix-rust p9-ws --root DIR --addr 127.0.0.1:7654` exposes the same
server over binary WebSocket frames for browser/v86 experiments. `wanix-rust
serve [DIR] [--listen HOST:PORT] [--bundle NAME] [--wanix-services]` serves static files with
COOP/COEP/CORS headers and reuses the binary WebSocket 9P handler on the same
listener, including the named `/.well-known/export9p` route, which is the first
Rust-native serve shape for browser/v86/VS Code experiments.
Normal `serve` handles HTTP and direct 9P WebSocket connections concurrently so
long-lived mounted clients do not block discovery or static assets; `--once`
remains a deterministic single-connection mode for tests and scripted demos.
`/.well-known/wanix.json` describes the direct binary 9P WebSocket route,
including base and Google.2 supported protocol strings, the optional bundle
hint, and the explicitly unimplemented Ethernet route so browser/v86/VS Code
clients can discover the current Rust serve contract.
`serve --wanix-services` switches the 9P export from a bare host directory to a
Wanix namespace that binds the host root at `/`, `#term`, and `#task` from a
service-root task context. The service table registers `noop` and `qjs`, so
direct 9P clients can allocate a QuickJS task with `#task/new/qjs`, set
`cmd`/`env`/`dir`, bind fds, and `ctl start` it inside the served namespace.
Discovery advertises those service roots and drivers through a `services`
object. The 9P server now preserves device-stream behavior for non-seekable fids
while keeping offset semantics for regular seekable files.
When launched with `--bundle fs9p`, `/?bundle=fs9p` returns a generated browser
filesystem smoke page that fetches discovery, opens the direct binary 9P
WebSocket route, negotiates 9P2000.L, and exposes list/read/write/rename/delete
controls against the served root. This is the browser-side 9P transport proof
for future workbench/VS Code filesystem integration, not the VS Code provider
itself.
The web workbench extension now also has a direct 9P filesystem backend that
matches the existing `WanixBridge` provider shape by consuming Rust serve
discovery and issuing 9P operations for `stat`, paginated directory listing,
file read/write, mkdir, rename, copy, and removal. It also registers bounded
client-side `wanix:/` file and text search providers when the served workbench
enables the matching VS Code proposed APIs. The classic MessagePort/CBOR
backend remains preferred when an embedding browser Wanix system supplies one.
When discovery advertises services, the Rust-served launcher passes `#task` and
`#term` paths into the extension and the direct 9P client can write existing
service files and keep `#term/...` streams open. This proves service reachability
over the workbench path. The same `--wanix-services` mode also exposes
`/.well-known/qjs-shell`, a Rust-owned terminal/session WebSocket route that
starts the bundled QuickJS shell as a Wanix task, pumps guest ready-IO while
browser terminal input arrives, returns terminal output as binary WebSocket
frames, and reports exit as a lifecycle text frame.
When launched with `--bundle workbench-fs9p`, `/?bundle=workbench-fs9p`
returns a generated filesystem-only VS Code web workbench launcher. The page
loads Code OSS and the `workbench/` extension package from the served root,
opens `wanix:/` as the workspace, uses VS Code IPC only to wake the extension
and pass a config-only absolute discovery URL, and does not supply the legacy
Wanix MessagePort filesystem bridge. The
intended backend remains the extension's Rust serve discovery path for direct
9P filesystem access.
This is a dev/demo launch path for local generated workbench assets, not a
packaged Code OSS distribution or a general task launcher.
The first browser smoke now proves the Rust-served page boots Code OSS to the
`wanix:/` workspace root, loads the served `wanix.workbench` extension, opens
the Rust direct 9P WebSocket, and populates Explorer from the served root.
`--wanix-services` adds a test-covered service namespace export, a qjs-backed
terminal route for the workbench pseudoterminal, and real one-shot qjs task
startup through `#task`. Richer task/session lifecycle controls remain
follow-ups.
When launched with `--bundle direct-v86`, `/?bundle=direct-v86` returns a small
browser page that fetches the discovery document and configures v86
`filesystem.proxy_url` with the Rust direct 9P WebSocket route. Discovery also
advertises direct-v86 boot hints: the default 9P-root Linux cmdline, memory
size, VGA memory size, virtio-console requirement, and embedded v86 asset
routes. Discovery also reports guest boot asset URLs found in the served root,
preferring `/boot/bzImage` over legacy `/bzImage` for the kernel and reporting a
present initrd route when one is available. Discovery marks direct-v86 boot
readiness by checking for a kernel and `/bin/init`, reporting missing required
markers for smoke automation. With `--bundle direct-v86`, Rust
serve owns the embedded `/v86/lib/libv86.mjs`, `/v86/lib/mod.js`,
`/v86/lib/offscreen.js`, wasm, and BIOS routes so the browser emulator runtime
does not have to live in the served root. The page accepts caller-supplied
kernel/initrd/cmdline URLs or query overrides, and `autostart=1` starts v86
after discovery/configuration so the URL can be used as a repeatable browser
boot smoke. The same generated page bridges v86
`virtio-console0-output-bytes` into a visible console textarea, sends
typed/pasted browser input back through `virtio-console0-input-bytes`, and
sends browser console size changes as `virtio-console0-resize`, matching the
guest's `hvc0` console path used by QEMU.
`wanix-rust rootfs --archive FILE.tgz --out DIR` extracts a gzipped tar guest
root into a missing or empty directory, rejects unsafe archive paths, validates
the shared VM boot markers (`/boot/bzImage` or `/bzImage`, plus `/bin/init`),
and prints ready-to-run QEMU and direct-v86 serve commands. This command is the
Rust-side bridge from `extras/dist/alpine-linux.tgz`-style artifacts to both VM
entrypoints; it prepares a directory but does not build the archive or manage VM
lifecycle.
`wanix-rust qemu --root DIR` prints a shell-quoted native QEMU/KVM virtio-9p
command for the same Linux guest/rootfs shape, discovering `/boot/bzImage` or
legacy `/bzImage` from the guest root unless `--kernel PATH` overrides it. The
command uses base `9p2000.L` root flags and `hvc0` virtconsole by default,
offers `--cmdline` and repeatable `--append` for guest boot tuning, and accepts
`--exec` as an explicit foreground launch mode. Exec mode spawns the same
validated argv, lets QEMU inherit native stdin/stdout/stderr for `-nographic`
console ownership, and returns QEMU's exit status; richer VM lifecycle,
rootfs build automation, signal policy, and network bridging remain follow-ups.
`/.well-known` routes are reserved for protocol endpoints;
`/.well-known/ethernet` is explicitly unimplemented until the qemu/vnet bridge
lands. Listener commands accept `--once` for tests and scripted demos.
`p9-stdio` and the `serve` well-known WebSocket route both have compatibility
probe smokes for auth, mknod, hard-link, xattr, legacy rename, and legacy
remove requests, and the serve WebSocket path has a Google.2 `Twalkgetattr`
smoke, pinning the externally visible contract used by
Linux/v86/editor clients.

## Code Quality Guardrails

Prefer modules under 250-350 non-test lines. Split responsibility-heavy modules
before they become difficult to review. Do not hold a namespace or filesystem
lock while calling into another filesystem.

Use explicit Rust types for public contracts. Avoid public raw `i32` flags,
file descriptors, rights, or modes where newtypes/builders make the trust
boundary clearer.

Required checks before a cycle commit:

```sh
cargo fmt --package wanix-9p --package wanix-cli --package wanix-fs --package wanix-protocol --package wanix-qjs --package wanix-qjs-engine --package wanix-task --package wanix-term --package wanix-vfs --package wanix-wasi --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

## ADR Index

Keep this index to active architecture, API, format, trust-boundary, and workflow
contracts. Deleted ADR numbers remain in git history; do not re-add milestone
proofs or progress-journal records as active ADRs.

- [ADR 0001](docs/adrs/0001-rust-native-wasmtime-runtime.md): Rust Wanix is the
  host/microkernel runtime, Wasmtime is the execution substrate, and Go is a
  migration oracle rather than a structure to copy.
- [ADR 0002](docs/adrs/0002-wanix-owned-wasi-semantics.md): Wanix owns live
  WASI Preview 1 filesystem, fd, clock, readiness, symlink, mutation, and
  service-path semantics through namespaces and task fds.
- [ADR 0003](docs/adrs/0003-quickjs-vm-image-snapshots.md): QuickJS snapshots
  are VM images; Wanix host resources and lifecycle policy are reattached at
  create/restore time.
- [ADR 0005](docs/adrs/0005-crate-boundaries-and-dependency-graph.md): Crate
  boundaries keep core Wanix contracts independent from QuickJS, Wasmtime,
  protocol transports, and composition-layer policy.
- [ADR 0009](docs/adrs/0009-quickjs-tasks-use-wanix-process-semantics.md):
  QuickJS is the execution engine inside `qjs` Wanix tasks while Wanix owns task
  identity, argv/env/cwd, fds, stdio, and exit state.
- [ADR 0012](docs/adrs/0012-quickjs-libc-std-fixture.md): The checked-in
  QuickJS fixture and `qjs:std`/`qjs:os` namespace modules are a compatibility
  boundary; virtual files are engine-only fixture support.
- [ADR 0017](docs/adrs/0017-explicit-host-directory-qjs-mounts.md): Host files
  enter Wanix only through explicit rooted `LocalFs` mounts with escape checks.
- [ADR 0040](docs/adrs/0040-quickjs-immediate-event-loop-turns.md): QuickJS
  timers, ready-IO turns, interrupt budgets, and memory limits are bounded host
  execution policy, not a scheduler or serialized task state.
- [ADR 0050](docs/adrs/0050-rust-terminal-device-and-shell-lifecycle.md):
  `wanix-term`, `qjs-term`, native `qjs-shell`, and served qjs-shell sessions
  share one terminal device and shell lifecycle contract.
- [ADR 0058](docs/adrs/0058-rust-9p-protocol-framing.md): `wanix-protocol` and
  `wanix-9p` own the Rust 9P frame, codec, fid, metadata, mutation, and
  compatibility contract across transports.
- [ADR 0068](docs/adrs/0068-rust-serve-http-websocket-9p.md): Rust `serve`
  owns local HTTP, discovery, direct 9P WebSocket, service namespace, and
  qjs-shell route policy for browser/v86/workbench clients.
- [ADR 0079](docs/adrs/0079-rust-serve-direct-v86-bundle.md): Direct v86 is a
  browser/emulator handoff that consumes Rust serve discovery, direct 9P, boot
  assets, hvc0 console, and smoke-readiness hints.
- [ADR 0088](docs/adrs/0088-native-qemu-virtio9p-handoff-command.md): Native
  QEMU support is a validated virtio-9p handoff command plus explicit
  foreground `--exec`, not a VM manager.
- [ADR 0092](docs/adrs/0092-serve-fs9p-browser-filesystem-bundle.md): Browser
  filesystem and workbench integration uses Rust serve discovery, direct 9P,
  `wanix:` filesystems, and `#task`/`#term` service workflows.

## Retired Bridge Notes

- The temporary `globalThis.Wanix` JavaScript helper API is retired. Guest code
  should use `qjs:std`, `qjs:os`, `scriptArgs`, stdio, and `#task` service
  files.
- The read-only virtual WASI projection is retired as a Wanix runtime path.
  Engine-level read-only virtual files may remain only as isolated fixture
  support under ADR 0012.
- Host line-discipline shell framing is retired. Native raw mode feeds bytes
  through the terminal device and lets the guest QuickJS shell own echo,
  editing, newline handling, and Ctrl-D behavior under ADR 0050.

## ADR Workflow

Treat ADRs like code. Add or update one only for durable architecture, API,
format, trust-boundary, or workflow decisions. When touching a topic, review the
related ADRs at the same time and consolidate, delete, or clearly retire records
that no longer describe the current direction.

Milestone proofs, fixture rebuild notes, per-syscall or per-operation coverage,
CLI/demo slices, and smoke-test progress belong in tests, examples,
current-state docs, and commit messages. Routine cargo command lists live in the
code-quality guardrails above unless the workflow itself changes.

## Cycle Rules

Prefer the highest-leverage externally visible capability or demo outcome.
Use cleanup only when it unblocks that outcome, protects a trust boundary,
preserves compatibility, or fixes a major review finding. Commit each completed
cycle before starting the next one. Every roughly five feature commits, run a
review/cleanup pass over the changes since the last cleanup pass before adding
more feature work.

## Queued Follow-ups

- Current `wanix-qjs` and `wanix-cli` tests cache the bundled QuickJS runner per
  test process; keep production runner caching out of scope unless it becomes an
  intentional runtime decision.
- Split large `wanix-qjs`, `wanix-cli`, and `wanix-wasi` modules before adding
  broad new behavior.
- Continue `qjs-shell` interactivity with signal-driven resize wakeups,
  cancellation, and richer terminal/session lifecycle control.
- Decide the auth/WebSocket policy needed for browser v86 and VS Code
  integration, then extend the direct-v86 route into a complete qemu/v86 bundle,
  `/.well-known/ethernet`, vnet, and VS Code routes on the Rust `serve`
  endpoint.
- Add backing contracts for 9P special files, hard links, or extended
  attributes only when a Linux/v86/editor workflow proves they are required.
