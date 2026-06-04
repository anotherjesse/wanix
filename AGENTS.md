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

## Current Capability Map

Keep this section concise. Detailed walkthrough output belongs in
`rust-walkthrough.md`; implementation history belongs in commit messages and
tests.

- `wanix-rust qjs main.js`: JavaScript runs outside Chrome as a Wanix `qjs`
  task with live Wanix-backed WASI, namespace access, stdio/fds, env/cwd/cmd,
  observable exit status, and `#task` service files.
- `wanix-rust qjs-term main.js` and `wanix-rust qjs-shell`: terminal-backed
  `qjs` tasks bind fd 0/1/2 through `#term/<id>/program`; native cooked/raw
  shell modes and served shell sessions use the same `#term` device contract,
  with a small `cd`/`ls`/`cat`/`write`/`mkdir`/`rm`/`rmdir`/`mv`/`cp`
  filesystem command set and synchronous child `qjs` task launches with
  namespace stdin redirection for demos.
- `wanix-rust p9-stdio`, `p9-listen`, `p9-ws`, and `serve`: the Rust 9P server
  exports Wanix filesystems over process, TCP, WebSocket, and HTTP composition
  layers, with binary protocol traffic kept separate from diagnostics.
- `serve --wanix-services`: exports a Wanix namespace containing the served
  root, `#task`, and `#term`; direct 9P clients can allocate/start `noop` or
  `qjs` tasks and attach terminal resources through service files.
- `serve --bundle fs9p`: browser filesystem client over direct 9P.
- `serve --bundle workbench-fs9p`: local Code OSS/workbench launch path where
  the bootstrap passes the discovered direct-9P route into the extension,
  negotiates Google.2 `walkgetattr` when available, browses and mutates
  `wanix:/` over direct 9P, and can open qjs-backed terminal sessions when
  services are enabled. Direct terminal disposal writes `close` through
  `#term/<id>/ctl`.
- `serve --bundle direct-v86`: browser v86 handoff over Rust serve discovery,
  direct 9P, boot-asset hints, hvc0 console bridging, and autostart-friendly
  launch hooks. The generated page reports rootfs handoff status and, for
  trusted loopback clients, exposes `window.wanixRootfsHandoff` plus copyable
  QEMU/direct-v86 serve commands.
- `wanix-rust rootfs --archive FILE.tgz --out DIR`: prepares a guest root,
  rejects unsafe archive paths, validates VM boot markers, and emits shell or
  `wanix-rootfs.v1` JSON handoffs for QEMU and direct-v86 without owning rootfs
  build or VM lifecycle.
- `/.well-known/rootfs.json`: when loopback clients access `serve` on a
  prepared guest root, exposes the same `wanix-rootfs.v1` handoff for trusted
  local browser/editor/VM launchers.
- `wanix-rust qemu --root DIR`: validates the same guest-root shape and emits a
  shell or `wanix-qemu-virtio9p.v1` JSON handoff with discovered or explicit
  initrd support; `--exec` is an explicit foreground launch, not a Wanix VM
  supervisor.

The biggest missing pieces remain interactive shell/session depth, broader
Linux/v86/editor 9P compatibility, QEMU/v86 boot workflows, and Rust
serve/workbench/VS Code integration. Ethernet/vnet and public auth remain
explicitly unimplemented trust-boundary work.

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

Keep this index to active architecture, API, format, trust-boundary, and
workflow contracts. The active set is intentionally small and consecutive; old
milestone numbers remain in git history, and milestone proofs or
progress-journal records should not be restored as active ADRs.

- [ADR 0001](docs/adrs/0001-rust-native-wasmtime-runtime.md): Rust Wanix is the
  host/microkernel runtime, Wasmtime is the execution substrate, Go is a
  migration oracle, and workspace crates stay layered around Wanix-owned
  contracts.
- [ADR 0002](docs/adrs/0002-quickjs-wasi-task-runtime.md): QuickJS runs as a
  Wanix `qjs` task; Wanix owns task identity, live WASI semantics, fd/service
  state, snapshots reattachment, fixture boundaries, and bounded guest
  execution policy.
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

## Runtime Guardrails

These are current guardrails for keeping retired bridge ideas out of new work:

- Guest JavaScript should use `qjs:std`, `qjs:os`, `scriptArgs`, stdio, env, and
  service files instead of `globalThis.Wanix` helpers.
- Wanix runtime paths should use live Wanix-backed WASI providers; engine
  read-only virtual files are fixture support only.
- Raw shell input should flow through `#term`; guest shells own echo, simple
  editing, newline handling, Ctrl-D, and command dispatch.
- Rust-served workbench paths should pass discovered direct-9P routes into the
  extension; the MessagePort/CBOR bridge is browser-embedded compatibility only.

## ADR Workflow

Treat ADRs like code. Add or update one only for durable architecture, API,
format, trust-boundary, or workflow decisions. When touching a topic, review the
related ADRs at the same time and consolidate, delete, or clearly retire records
that no longer describe the current direction.

The root ADR index above is the active Wanix decision set: runtime boundary,
QuickJS/WASI task boundary, terminals, 9P, and serve/client handoffs. Prefer
revising one of those records before adding a new one. Imported prototype ADR
archives should be consolidated into topic docs or deleted; do not let nested
crates regrow progress-journal ADR series. Do not keep replacement ledgers
inside active ADRs; git history already records which milestone notes were
removed.

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

- Current `wanix-qjs` and `wanix-cli` tests cache the bundled QuickJS runner per
  test process; keep production runner caching out of scope unless it becomes an
  intentional runtime decision.
- Split large `wanix-qjs`, `wanix-cli`, and `wanix-wasi` modules before adding
  broad new behavior.
- Continue `qjs-shell` interactivity with signal-driven resize wakeups,
  foreground child-task terminal ownership, cancellation, command execution
  beyond the current built-ins and synchronous `qjs` launcher with file stdin
  redirection, and richer terminal/session lifecycle control.
- Decide the auth/WebSocket policy needed for browser v86 and VS Code
  integration, then extend the direct-v86 route into a complete qemu/v86 bundle,
  `/.well-known/ethernet`, vnet, and VS Code routes on the Rust `serve`
  endpoint.
- Add backing contracts for 9P special files or extended attributes only when a
  Linux/v86/editor workflow requires them.
