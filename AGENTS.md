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
When given `--event-loop-ms`, native `qjs-shell` also pumps bounded QuickJS
event-loop work after each terminal input batch, so delayed shell output can
surface before the next scripted input line.
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
discovery and issuing 9P operations for `stat`, directory listing, file
read/write, mkdir, rename, copy, and removal. The classic MessagePort/CBOR
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
startup through `#task`, but search providers and richer task/session lifecycle
controls remain follow-ups.
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
- [ADR 0007](docs/adrs/0007-workspace-local-rust-quality-gate.md): Formatting
  checks enumerate current Wanix workspace crates; clippy and tests remain
  workspace-wide.
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
  `qjs-shell --raw` first disabled host canonical input/echo and used a small
  host line discipline; ADR 0091 supersedes its guest input boundary.
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
- [ADR 0070](docs/adrs/0070-go-like-rust-serve-cli.md):
  Rust `serve` defaults to a demo-friendly current-directory root, supports
  positional directories, `--listen`, and bundle URL reporting.
- [ADR 0071](docs/adrs/0071-rust-serve-discovery-document.md):
  Rust `serve` exposes `/.well-known/wanix.json` so browser/v86/VS Code clients
  can discover direct 9P and reserved Ethernet routes.
- [ADR 0072](docs/adrs/0072-9p-statfs-mount-probe.md):
  `wanix-9p` answers `Tstatfs` with conservative synthetic filesystem stats so
  Linux/v86 clients can complete mount probes.
- [ADR 0073](docs/adrs/0073-9p-symlink-readlink.md):
  `wanix-9p` exposes Wanix-owned symlink creation and readlink semantics through
  9P2000.L `Tsymlink` and `Treadlink`.
- [ADR 0074](docs/adrs/0074-9p-flush-fsync.md):
  `wanix-9p` acknowledges `Tflush` and validates `Tfsync` fids so external
  clients can pass common sync and cancelation probes.
- [ADR 0075](docs/adrs/0075-9p-setattr-size-times.md):
  `wanix-9p` supports the `Tsetattr` size and access/modification time subset
  while returning explicit unsupported errors for ctime changes.
- [ADR 0076](docs/adrs/0076-9p-lock-probe-compatibility.md):
  `wanix-9p` answers `Tlock` and `Tgetlock` as compatibility probes without a
  persistent advisory-lock manager.
- [ADR 0077](docs/adrs/0077-9p-open-append-semantics.md):
  `wanix-9p` stores `O_APPEND` as opened fid state so writes append at EOF
  regardless of 9P write offsets.
- [ADR 0078](docs/adrs/0078-9p-setattr-permissions.md):
  `wanix-9p` routes `Tsetattr(PERMISSIONS)` through Wanix filesystem permission
  mutation for chmod-compatible mounted workflows.
- [ADR 0079](docs/adrs/0079-rust-serve-direct-v86-bundle.md):
  `wanix-rust serve --bundle direct-v86` serves a browser page that wires the
  discovery document's direct 9P WebSocket route into v86 `filesystem.proxy_url`.
- [ADR 0080](docs/adrs/0080-9p-virtual-ownership.md):
  `wanix-9p` stores uid/gid as session-virtual metadata so mounted clients can
  observe chown-style changes without adding a filesystem-wide ownership API.
- [ADR 0081](docs/adrs/0081-9p-compatibility-probe-codecs.md):
  `wanix-9p` decodes auth, mknod, hard-link, and xattr probes as typed 9P
  requests and returns deliberate errno responses until backing contracts
  exist.
- [ADR 0082](docs/adrs/0082-9p-legacy-rename-remove.md):
  `wanix-9p` supports legacy fid-oriented `Trename` and `Tremove`, including
  rename fid rebasing and remove clunk semantics, for mounted-client
  compatibility.
- [ADR 0083](docs/adrs/0083-serve-direct-v86-boot-contract.md):
  Rust `serve --bundle direct-v86` advertises and applies the 9P-root Linux
  boot cmdline and virtio-console defaults needed by the v86 handoff.
- [ADR 0084](docs/adrs/0084-serve-embedded-direct-v86-assets.md):
  Rust `serve --bundle direct-v86` serves embedded v86 module, wasm, and BIOS
  assets and advertises those routes in discovery.
- [ADR 0085](docs/adrs/0085-serve-direct-v86-boot-asset-discovery.md):
  Rust direct-v86 discovery reports kernel and initrd routes found in the served
  root, preferring the guest-root `/boot/bzImage` layout.
- [ADR 0086](docs/adrs/0086-concurrent-rust-serve-clients.md):
  Normal Rust `serve` accepts concurrent HTTP and direct 9P WebSocket clients
  while `--once` stays single-connection for deterministic smokes.
- [ADR 0087](docs/adrs/0087-9p-google2-walkgetattr.md):
  `wanix-9p` negotiates Google.2 9P extensions for `Tflushf` and
  `Twalkgetattr` while keeping v86's default mount contract on base 9P.
- [ADR 0088](docs/adrs/0088-native-qemu-virtio9p-handoff-command.md):
  `wanix-rust qemu` prints a validated QEMU/KVM virtio-9p handoff command
  without becoming a VM process supervisor yet.
- [ADR 0089](docs/adrs/0089-qemu-root-kernel-discovery-and-cmdline-options.md):
  `wanix-rust qemu` discovers guest-root kernels and exposes cmdline
  override/append options while staying a print-only handoff.
- [ADR 0090](docs/adrs/0090-direct-v86-hvc0-console-bridge.md):
  Rust direct-v86 pages expose the guest `hvc0` virtio-console stream for
  visible browser boot and shell interaction.
- [ADR 0091](docs/adrs/0091-qjs-shell-raw-byte-pump.md):
  `qjs-shell --raw` feeds native bytes directly to the guest shell so QuickJS
  owns visible echo/editing behavior.
- [ADR 0092](docs/adrs/0092-serve-fs9p-browser-filesystem-bundle.md):
  `wanix-rust serve --bundle fs9p` serves a browser 9P filesystem smoke page
  for future workbench/VS Code filesystem integration.
- [ADR 0093](docs/adrs/0093-workbench-direct-9p-filesystem-backend.md):
  The web workbench can back its existing `wanix:` filesystem provider with
  Rust serve discovery and direct 9P while leaving task/terminal semantics out.
- [ADR 0094](docs/adrs/0094-serve-workbench-fs9p-bundle.md):
  Rust `serve` provides a filesystem-only VS Code web workbench launch surface
  for the direct 9P workbench path as a dev/demo path.
- [ADR 0095](docs/adrs/0095-serve-wanix-service-namespace.md):
  `serve --wanix-services` exports `#task` and `#term` through a Wanix namespace
  over direct 9P as the service foundation used by the later qjs-shell route.
- [ADR 0096](docs/adrs/0096-serve-qjs-shell-terminal-route.md):
  `serve --wanix-services` exposes a qjs-shell WebSocket route so the workbench
  can drive a QuickJS-backed Wanix task through terminal bytes.
- [ADR 0097](docs/adrs/0097-serve-qjs-task-service-driver.md):
  `serve --wanix-services` registers a real `qjs` task driver and exports
  `#task` from a service-root task context so direct 9P clients can start JS.
- [ADR 0098](docs/adrs/0098-workbench-qjs-task-command.md):
  The Rust-served workbench can run the active `wanix:` JavaScript file as a
  `qjs` Wanix task by driving `#task` and `#term` over direct 9P.
- [ADR 0099](docs/adrs/0099-serve-qjs-shell-idle-pump.md):
  The served qjs-shell WebSocket route pumps bounded QuickJS event-loop work
  while idle so delayed terminal output can reach the workbench without another
  browser input frame.
- [ADR 0100](docs/adrs/0100-direct-v86-boot-smoke-readiness.md):
  The direct-v86 bundle supports `autostart=1`, boot logs, lifecycle status,
  hvc0 resize, and discovery readiness markers for repeatable browser VM smokes.
- [ADR 0101](docs/adrs/0101-qemu-exec-foreground-supervision.md):
  `wanix-rust qemu --exec` foreground-spawns the validated QEMU argv while
  preserving the print-only default command contract.

## Superseded ADRs

- [ADR 0006](docs/adrs/0006-interim-quickjs-wanix-host-api.md): The temporary
  `Wanix` JavaScript host API was removed after qjs std/os, `scriptArgs`,
  `#task`, and live Wanix-backed WASI took over.
- [ADR 0008](docs/adrs/0008-quickjs-namespace-modules-and-virtual-wasi-projection.md):
  QuickJS namespace module loading remains current, but the read-only virtual
  WASI projection bridge is superseded by live Wanix-backed WASI.

## Cycle Rules

Prefer the highest-leverage externally visible capability or demo outcome.
Use cleanup only when it unblocks that outcome, protects a trust boundary,
preserves compatibility, or fixes a major review finding. Commit each completed
cycle before starting the next one.

Treat ADRs like code. Before adding one, check whether the change is a durable
architecture, API, format, or workflow decision instead of a milestone note or
implementation diary. Prefer current-state docs plus tests for routine CLI/demo
slices, and when adding or touching ADRs, prune, delete, or clearly mark
superseded records so `AGENTS.md` does not describe old bridges as current
direction.

## Queued Follow-ups

- Current `wanix-qjs` and `wanix-cli` tests cache the bundled QuickJS runner per
  test process; keep production runner caching out of scope unless it becomes an
  intentional runtime decision.
- Split large `wanix-qjs`, `wanix-cli`, and `wanix-wasi` modules before adding
  broad new behavior.
- Run a dedicated ADR librarian pass: delete or consolidate superseded bridge
  records, fixture rebuild notes, per-WASI-call records, and per-9P-op records
  into subsystem-level decisions.
- Continue `qjs-shell` interactivity with a native fd-aware idle loop that can
  pump guest output while blocked waiting for process stdin, plus signal
  handling and live native resize propagation.
- Decide the auth/WebSocket policy needed for browser v86 and VS Code
  integration, then extend the direct-v86 route into a complete qemu/v86 bundle,
  `/.well-known/ethernet`, vnet, and VS Code routes on the Rust `serve`
  endpoint.
- Add backing contracts for 9P special files, hard links, or extended
  attributes only when a Linux/v86/editor workflow proves they are required.
