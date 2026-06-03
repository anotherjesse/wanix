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
- `wanix-vfs`: Plan 9-style namespace binding and resolution.
- `wanix-task`: task model, `#task`, fd table, and driver registry.
- `wanix-wasi`: custom WASI Preview 1 imports backed by Wanix namespaces and
  task fds supplied by adapter configuration.
- `wanix-qjs-engine`: Wasmtime-hosted QuickJS/WASI engine mechanics relocated
  from the `rust-wasi-quickjs` prototype, keeping the existing Rust import path
  while using the workspace-local QuickJS WASM fixture.
- `wanix-qjs`: QuickJS/WASI task driver that adapts the engine crate to Wanix
  task semantics.
- `wanix-cli`: native CLI and demo runner.
- Future `wanix-protocol`: 9P, CBOR/RPC, HTTPFS, and R2FS protocol pieces.

## Dependency Direction

Keep lower-level crates independent from runtime engines:

```text
wanix-fs
  -> wanix-vfs
  -> wanix-task

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

## Code Quality Guardrails

Prefer modules under 250-350 non-test lines. Split responsibility-heavy modules
before they become difficult to review. Do not hold a namespace or filesystem
lock while calling into another filesystem.

Use explicit Rust types for public contracts. Avoid public raw `i32` flags,
file descriptors, rights, or modes where newtypes/builders make the trust
boundary clearer.

Required checks before a cycle commit:

```sh
cargo fmt --package wanix-cli --package wanix-fs --package wanix-qjs --package wanix-qjs-engine --package wanix-task --package wanix-vfs --package wanix-wasi --check
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
