# AGENTS.md

## Rust Wanix Port

North star: build a Rust-native Wanix core that runs outside Chrome, with
Wasmtime as the execution substrate and QuickJS/WASI as the first serious task
runtime. Browser support becomes a frontend or deployment option, not the
runtime foundation.

## First Demo Target

Run JavaScript outside Chrome with access to a Wanix namespace.

## Crate Shape

- `wanix-fs`: filesystem traits, metadata, errors, path rules, and in-memory
  fixtures.
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
- [ADR 0006](docs/adrs/0006-interim-quickjs-wanix-host-api.md): QuickJS gets a
  narrow temporary `Wanix` host API until Wanix-backed WASI imports are wired.
- [ADR 0007](docs/adrs/0007-workspace-local-rust-quality-gate.md): Formatting
  checks enumerate Wanix crates while sibling prototypes remain path
  dependencies.
- [ADR 0008](docs/adrs/0008-quickjs-namespace-modules-and-virtual-wasi-projection.md):
  QuickJS ES modules load from Wanix namespaces while read-only virtual
  projection remains an interim namespace-demo path.
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
  Broad libc regular-file open requests are projected to the Wanix-enforced
  file rights reported on the opened fd.
- [ADR 0014](docs/adrs/0014-wasi-service-paths-root-relative.md):
  WASI paths beginning with `#task` stay rooted at the Wanix task service root
  even when the ordinary root preopen maps to a task cwd.

## Cycle Rules

Prefer the highest-leverage externally visible capability or demo outcome.
Use cleanup only when it unblocks that outcome, protects a trust boundary,
preserves compatibility, or fixes a major review finding. Commit each completed
cycle before starting the next one.

## Queued Follow-ups

- If QuickJS-heavy tests become too slow, add a test-only
  `OnceLock<Result<Arc<QuickJsRunner>, String>>` fixture in `wanix-qjs`, adjust
  qjs task-driver tests to clone the cached runner, and consider a similar
  injection seam for `wanix-cli` qjs tests. Keep production runner caching out
  of scope unless it becomes an intentional runtime decision.
