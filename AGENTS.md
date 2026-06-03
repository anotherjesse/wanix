# AGENTS.md

## Rust Wanix Port

North star: build a Rust-native Wanix core that runs outside Chrome, with
Wasmtime as the execution substrate and QuickJS/WASI as the first serious task
runtime. Browser support becomes a frontend or deployment option, not the
runtime foundation.

## First Demo Target

Run JavaScript outside Chrome with access to a Wanix namespace.

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
- Future protocol work: typed 9P operations and server/client adapters, plus
  CBOR/RPC, HTTPFS, and R2FS protocol pieces when those integrations need them.

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

## First Vertical Slice

The first real demo should be `wanix-rust qjs main.js`: JavaScript runs outside
Chrome, reads and writes files through a Wanix namespace, prints through a
stdio/console shim, and exits with an observable status. A strong follow-up demo
should read `#task/self/id` from JavaScript to prove task context crosses into
QuickJS.

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
`qjs-shell --raw` adds native raw-mode setup plus host-side local echo and simple
line editing while still delivering complete lines to the guest task.
`qjs-term --resize-after-eval COLSxROWS` sends a deterministic post-eval
resize event to `#term/<id>/winch` as `columns rows\n`, proving QuickJS tasks
can observe terminal resize broadcasts through live Wanix-backed fd readiness.

The next 9P-facing demo target is wiring the native and browser listeners into
serve/v86 experiments so external clients can browse a Wanix namespace. The
server core already negotiates 9P2000.L, attaches, walks, opens regular files
and read-only directories, reads/writes regular files, lists directories with
`Treaddir`, clunks fids, reports metadata with `Tgetattr`, and handles core
mutation ops with `Tlcreate`, `Tmkdir`, `Trenameat`, and `Tunlinkat`; it also
serves encoded request/response frames through a synchronous stream loop.
`wanix-rust p9-stdio --root DIR` exposes that server over process
stdin/stdout, with stdout reserved for binary 9P responses and stderr reserved
for human-readable CLI or transport errors. `wanix-rust p9-listen --root DIR
--addr 127.0.0.1:5640` exposes the same `LocalFs` export over a native TCP
listener. `wanix-rust p9-ws --root DIR --addr 127.0.0.1:7654` exposes the same
server over binary WebSocket frames for browser/v86 experiments. `wanix-rust
serve --root DIR --addr 127.0.0.1:7654` serves static files with
COOP/COEP/CORS headers and reuses the binary WebSocket 9P handler on the same
listener, including the named `/.well-known/export9p` route, which is the first
Rust-native serve shape for browser/v86/VS Code experiments. `/.well-known`
routes are reserved for protocol endpoints; `/.well-known/ethernet` is
explicitly unimplemented until the qemu/vnet bridge lands. Listener commands
accept `--once` for tests and scripted demos.

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

- [ADR 0001](docs/adrs/0001-rust-native-wasmtime-runtime.md): Rust Wanix is the
  host/microkernel and Wasmtime is the execution substrate.
- [ADR 0002](docs/adrs/0002-wanix-owned-wasi-semantics.md): Wanix owns WASI
  filesystem and fd semantics instead of delegating them to host WASI.
- [ADR 0003](docs/adrs/0003-quickjs-vm-image-snapshots.md): QuickJS snapshots
  are VM images and host resources must be reattached explicitly.
- [ADR 0004](docs/adrs/0004-go-as-migration-oracle.md): The Go implementation
  is a migration oracle, not a structure to copy blindly.
- [ADR 0005](docs/adrs/0005-crate-boundaries-and-dependency-graph.md): Crate
  boundaries keep core Wanix contracts independent from Wasmtime and QuickJS.
- [ADR 0006](docs/adrs/0006-interim-quickjs-wanix-host-api.md): The temporary
  `Wanix` JavaScript host API is fully superseded by qjs std/os, `scriptArgs`,
  `#task`, and live Wanix-backed WASI.
- [ADR 0007](docs/adrs/0007-workspace-local-rust-quality-gate.md): Formatting
  checks enumerate Wanix crates while sibling prototypes remain path
  dependencies.
- [ADR 0008](docs/adrs/0008-quickjs-namespace-modules-and-virtual-wasi-projection.md):
  QuickJS ES modules load from Wanix namespaces; the old read-only virtual
  projection bridge is superseded by live Wanix-backed WASI.
- [ADR 0009](docs/adrs/0009-quickjs-tasks-use-wanix-process-semantics.md):
  QuickJS is the execution engine inside `qjs` Wanix tasks, not a separate
  process model.
- [ADR 0010](docs/adrs/0010-workspace-local-quickjs-engine-crate.md):
  QuickJS/Wasmtime engine mechanics live in a Wanix workspace crate so Wanix can
  evolve the WASI boundary directly.
- [ADR 0011](docs/adrs/0011-live-quickjs-wasi-host-provider.md): Live
  QuickJS WASI hooks are runtime host state carried by create/restore options,
  while deterministic host config stays cloneable and comparable.
- [ADR 0012](docs/adrs/0012-quickjs-libc-std-fixture.md): The checked-in
  QuickJS fixture initializes `qjs:std`/`qjs:os` so guest JavaScript can reach
  Wanix-backed WASI stdio.
- [ADR 0013](docs/adrs/0013-preview1-regular-file-rights-projection.md):
  Broad libc file open requests are projected to the Wanix-enforced rights
  reported on the opened fd.
- [ADR 0014](docs/adrs/0014-wasi-service-paths-root-relative.md):
  WASI paths beginning with `#task` stay rooted at the Wanix task service root
  even when the ordinary root preopen maps to a task cwd.
- [ADR 0015](docs/adrs/0015-task-cmd-shell-argv-format.md): `#task/cmd`
  writes use a shell-quoted argv format so file-controlled tasks preserve
  spaces, quotes, and empty arguments.
- [ADR 0016](docs/adrs/0016-quickjs-task-runtime-restore-lifecycle.md):
  `QuickJsTaskRuntime` owns create/restore lifecycle for qjs task VMs while
  Wanix host resources are reattached outside the snapshot image.
- [ADR 0017](docs/adrs/0017-explicit-host-directory-qjs-mounts.md):
  Native qjs demos expose host files only through explicit rooted directory
  mounts into a Wanix namespace.
- [ADR 0018](docs/adrs/0018-qjs-restore-process-context-options.md):
  `qjs-restore` configures before and after task argv/env separately so restore
  demos expose the VM-versus-host-state boundary.
- [ADR 0019](docs/adrs/0019-task-ctl-bind-shell-words.md): `#task/ctl bind`
  operands use the same shell-word grammar as `#task/cmd`, preserving Wanix
  paths with spaces when wiring fds.
- [ADR 0020](docs/adrs/0020-qjs-wasi-dynamic-fd-mirroring.md): qjs task
  runtimes mirror dynamic regular-file WASI fds into the Wanix task fd table at
  the same numeric fd.
- [ADR 0021](docs/adrs/0021-native-qjs-stdin-sources.md): `wanix-rust qjs`
  accepts text, host-file, or native stdin sources and installs them as Wanix
  task fd 0.
- [ADR 0022](docs/adrs/0022-task-fd-service-open-handoff.md): opening
  `#task/<id>/fd/<n>` captures a shared open-file handle so fd binds remain
  usable after the source task closes its fd.
- [ADR 0023](docs/adrs/0023-wasi-file-unlink-through-wanix-namespaces.md):
  `qjs:os.remove(...)` reaches Wanix-owned `PATH_UNLINK_FILE` semantics through
  namespaces and live WASI providers.
- [ADR 0024](docs/adrs/0024-persistent-qjs-snapshot-cli.md): `qjs-snapshot`
  and `qjs-resume` persist QuickJS VM images while Wanix host resources are
  reattached per invocation.
- [ADR 0025](docs/adrs/0025-wasi-directory-create-through-wanix-namespaces.md):
  `qjs:os.mkdir(...)` reaches Wanix-owned `PATH_CREATE_DIRECTORY` semantics
  through namespaces and live WASI providers.
- [ADR 0026](docs/adrs/0026-wasi-directory-remove-through-wanix-namespaces.md):
  `qjs:os.remove(...)` removes empty directories through Wanix-owned
  `PATH_REMOVE_DIRECTORY` semantics and live WASI providers.
- [ADR 0027](docs/adrs/0027-wasi-rename-through-wanix-namespaces.md):
  `qjs:os.rename(...)` reaches same-filesystem Wanix namespace rename through
  live WASI providers.
- [ADR 0028](docs/adrs/0028-wasi-append-fdflag-through-wanix-files.md):
  `qjs:os.open(..., O_APPEND)` and `qjs:std.open(..., "a")` append through
  Wanix-owned WASI fdflags and shared file handles.
- [ADR 0029](docs/adrs/0029-wasi-runtime-append-fdflag-mutation.md):
  `fd_fdstat_set_flags(APPEND)` mutates shared Wanix fd state so
  `qjs:std.fdopen(fd, "a")` appends through live and mirrored task fds.
- [ADR 0030](docs/adrs/0030-wasi-path-timestamp-mutation.md):
  `qjs:os.utimes(...)` and `fd_filestat_set_times` reach Wanix-owned timestamp
  mutation and stat-visible atime/mtime through live WASI providers.
- [ADR 0031](docs/adrs/0031-wasi-timer-poll-oneoff.md):
  `qjs:os.sleep(...)` reaches timer-only Preview 1 `poll_oneoff`.
- [ADR 0032](docs/adrs/0032-wasi-directory-listing-through-quickjs.md):
  `qjs:os.readdir(...)` reaches Wanix-owned directory listing through live WASI
  providers and libc-style directory rights.
- [ADR 0033](docs/adrs/0033-wasi-fd-filestat-set-size.md):
  `fd_filestat_set_size` resizes open Wanix-backed regular files through live
  WASI providers; ADR 0039 later exposes it through QuickJS truncate helpers.
- [ADR 0034](docs/adrs/0034-wasi-timestamp-now-clock-policy.md):
  WASI `ATIM_NOW`/`MTIM_NOW` timestamp updates use the deterministic clock value
  carried by `WasiConfig`.
- [ADR 0035](docs/adrs/0035-wasi-poll-fd-readiness.md):
  `poll_oneoff` reports immediately-ready fd read/write events through live
  WASI providers while async scheduling remains future Wanix task work.
- [ADR 0036](docs/adrs/0036-wasi-symlink-metadata-lookup.md):
  WASI `path_filestat_get` lookup flags reach Wanix namespace metadata so live
  providers can distinguish final symlink metadata from followed targets.
- [ADR 0037](docs/adrs/0037-wasi-symlink-readlink-and-create.md):
  WASI `path_readlink` and `path_symlink` flow through live Wanix namespaces
  while link targets remain byte contents rather than normalized Wanix paths.
- [ADR 0038](docs/adrs/0038-quickjs-wasi-symlink-stdlib-fixture.md):
  QuickJS `qjs:os.lstat`, `readlink`, and `symlink` reach live Wanix-backed
  WASI providers without enabling QuickJS process APIs.
- [ADR 0039](docs/adrs/0039-quickjs-wasi-truncate-stdlib-fixture.md):
  QuickJS `qjs:os.truncate` and `ftruncate` reach Wanix-backed
  `fd_filestat_set_size` through live WASI providers.
- [ADR 0040](docs/adrs/0040-quickjs-immediate-event-loop-turns.md):
  QuickJS due async timers run through bounded immediate event-loop turns after
  Wanix `qjs` task evaluation.
- [ADR 0041](docs/adrs/0041-quickjs-ready-fd-handler-turn.md):
  QuickJS `setReadHandler`/`setWriteHandler` callbacks get one nonblocking
  ready-fd turn after Wanix `qjs` task evaluation.
- [ADR 0042](docs/adrs/0042-bounded-quickjs-future-timer-pump.md):
  QuickJS future timers can run after qjs task evaluation when the composition
  layer grants an explicit bounded wait budget.
- [ADR 0043](docs/adrs/0043-bounded-quickjs-ready-io-turns.md):
  QuickJS ready-fd handlers can run for an explicit fixed number of
  nonblocking turns after Wanix `qjs` task evaluation.
- [ADR 0044](docs/adrs/0044-bounded-quickjs-interval-timers.md):
  QuickJS self-clearing interval timers run inside the bounded future-timer pump
  without defining a general scheduler.
- [ADR 0045](docs/adrs/0045-qjs-interrupt-poll-budget.md):
  Wanix `qjs` tasks can stop CPU-bound JavaScript through an explicit QuickJS
  interrupt-poll budget without adding signals or a scheduler.
- [ADR 0046](docs/adrs/0046-qjs-memory-limit-policy.md):
  Wanix `qjs` tasks can apply an explicit QuickJS heap memory limit while
  preserving default unlimited behavior.
- [ADR 0047](docs/adrs/0047-snapshot-resume-memory-limit-reattachment.md):
  `qjs-snapshot` and `qjs-resume` reattach QuickJS heap limits as host policy
  instead of serializing them into VM snapshot files.
- [ADR 0048](docs/adrs/0048-snapshot-resume-interrupt-budget-reattachment.md):
  `qjs-snapshot` and `qjs-resume` reattach QuickJS interrupt-poll budgets as
  host policy instead of serializing them into VM snapshot files.
- [ADR 0049](docs/adrs/0049-snapshot-resume-event-loop-budget-reattachment.md):
  `qjs-snapshot` and `qjs-resume` reattach bounded future-timer and ready-IO
  budgets as host lifecycle policy instead of serializing them into VM
  snapshot files.
- [ADR 0050](docs/adrs/0050-rust-terminal-device-foundation.md):
  `wanix-term` implements the first Rust-native `#term` device contract for
  terminal/program byte flow, id allocation, and resize-event broadcast.
- [ADR 0051](docs/adrs/0051-terminal-backed-qjs-cli-demo.md):
  `wanix-rust qjs-term` runs a QuickJS task through `#term` fd bindings and
  returns the terminal transcript as the native CLI output.
- [ADR 0052](docs/adrs/0052-qjs-term-post-eval-input-feed.md):
  `qjs-term` post-eval feeds keep the task runtime alive after initial eval,
  feed terminal data, and pump ready-IO handlers after each input batch or
  streamed native stdin line.
- [ADR 0053](docs/adrs/0053-terminal-fd-readiness.md): Wanix fd readiness hooks
  let `#term` report readable only when terminal input or output is queued.
- [ADR 0054](docs/adrs/0054-qjs-term-native-output-streaming.md):
  `wanix-rust` uses a process-IO CLI entrypoint so `qjs-term` can stream
  terminal output during eval/feed/ready-IO slices.
- [ADR 0055](docs/adrs/0055-native-qjs-shell-command.md):
  `wanix-rust qjs-shell` runs the bundled QuickJS shell through the
  terminal-backed qjs task runtime as the direct native shell demo.
- [ADR 0056](docs/adrs/0056-qjs-shell-native-raw-mode.md):
  `qjs-shell --raw` disables host canonical input/echo when stdin is a TTY and
  uses a small host line discipline before feeding Wanix terminal lines.
- [ADR 0057](docs/adrs/0057-qjs-term-post-eval-resize-feed.md):
  `qjs-term --resize-after-eval` writes textual resize events to `#term/winch`
  and pumps ready-IO so QuickJS tasks can observe resize broadcasts.
- [ADR 0058](docs/adrs/0058-rust-9p-protocol-framing.md):
  `wanix-protocol` owns dependency-free 9P frame splitting, tag extraction, and
  version negotiation before Rust grows a Wanix-backed 9P server.
- [ADR 0059](docs/adrs/0059-basic-9p-operation-codecs.md):
  `wanix-protocol` owns typed codecs for the first server-facing 9P2000.L
  attach/walk/open/read/write/clunk/error operations.
- [ADR 0060](docs/adrs/0060-wanix-backed-9p-server-adapter.md):
  `wanix-9p` maps typed 9P frames onto Wanix filesystem fids and Linux errno
  replies while transports remain out of scope.
- [ADR 0061](docs/adrs/0061-9p-directory-read-cookies.md):
  `wanix-9p` serves `Treaddir` from Wanix directory entries using opaque
  one-based cookies that preserve the Go p9kit behavior.
- [ADR 0062](docs/adrs/0062-sync-9p-stream-transport-loop.md):
  `wanix-9p` serves decoded request frames from synchronous byte streams while
  keeping socket/listener policy outside the server core.
- [ADR 0063](docs/adrs/0063-stdio-9p-cli-bridge.md):
  `wanix-rust p9-stdio --root DIR` exposes a `LocalFs` root as a binary 9P
  request/response stream over native process stdio.
- [ADR 0064](docs/adrs/0064-9p-getattr-metadata.md):
  `wanix-9p` maps `Tgetattr` to Wanix metadata and returns fixed 9P2000.L
  `Rgetattr` payloads with POSIX file-type mode bits.
- [ADR 0065](docs/adrs/0065-native-tcp-9p-listener.md):
  `wanix-rust p9-listen --root DIR --addr HOST:PORT` exposes the Rust 9P
  server over native TCP, with `--once` for deterministic smoke tests.
- [ADR 0066](docs/adrs/0066-9p-mutation-operations.md):
  `wanix-9p` maps `Tlcreate`, `Tmkdir`, `Trenameat`, and `Tunlinkat` onto the
  existing Wanix filesystem mutation traits.
- [ADR 0067](docs/adrs/0067-browser-websocket-9p-bridge.md):
  `wanix-rust p9-ws --root DIR --addr HOST:PORT` exposes the Rust 9P server
  over binary WebSocket messages for browser/v86 experiments.
- [ADR 0068](docs/adrs/0068-rust-serve-http-websocket-9p.md):
  `wanix-rust serve --root DIR --addr HOST:PORT` combines static HTTP assets
  and binary WebSocket 9P export on one browser-facing listener.
- [ADR 0069](docs/adrs/0069-serve-well-known-routing.md):
  Rust `serve` reserves `/.well-known` protocol routes, maps
  `/.well-known/export9p` to direct binary 9P, and leaves Ethernet/vnet
  explicitly unimplemented.

## Cycle Rules

Prefer the highest-leverage externally visible capability or demo outcome.
Use cleanup only when it unblocks that outcome, protects a trust boundary,
preserves compatibility, or fixes a major review finding. Commit each completed
cycle before starting the next one.

## Queued Follow-ups

- Current `wanix-qjs` and `wanix-cli` tests cache the bundled QuickJS runner per
  test process; keep production runner caching out of scope unless it becomes an
  intentional runtime decision.
- Split large `wanix-qjs`, `wanix-cli`, and `wanix-wasi` modules before adding
  broad new behavior.
- Make `qjs-shell` genuinely interactive beyond line-oriented input by adding a
  host loop that can wait on native input and guest output concurrently, signal
  handling, and live native resize propagation.
- Decide the auth/WebSocket policy needed for browser v86 and VS Code
  integration, then wire qemu/v86 bundles, `/.well-known/ethernet`, vnet, and
  VS Code routes onto the Rust `serve` endpoint.
