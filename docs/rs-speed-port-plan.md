# Porting rs-speed features onto the `rust` branch

Status: **Phase A in progress.** Branch: `rust` (direct). Cache decision recorded by
**extending ADR 0002** (not a new ADR).

## Goal

Bring the additive performance/feature work from `origin/rs-speed` onto `origin/rust`
by *skillful porting* — adapting each change to `rust`'s refactored module layout
rather than cherry-picking or merging. Where the two branches solved the same problem
differently, keep whichever approach the investigation showed is better.

## Phased execution: A then B

Two phases. **Phase B does not start until Phase A is fully green, committed, reviewed, and
quality-checked.**

- **Phase A — Tier-1 integration (the port).** Land everything from rs-speed as a *Tier-1
  demo/bench harness*: shared WASI linker (wasm-scoped), the `WasiRunner`, memfs symlink,
  module cache (made an intentional decision via an ADR), the CLI `wasm` command, benchmarks
  and perf docs. The wasm runner is explicitly **not** a Wanix task driver yet — it is a
  free-standing runner + CLI demo proving the shared-VFS substrate and the compiled-vs-
  interpreted tier. End state: `just check` green, every cycle committed, one review/cleanup
  pass done.
- **Phase B — `.wasm` as a real Wanix task.** Only after A. Wrap the existing `WasiRunner` in
  a `WasmTaskDriver: TaskDriver` so a `.wasm` cmd becomes a first-class Wanix task (task fds,
  namespace, env/cwd/argv, observable exit, `#task` service files, fd mirroring per ADR 0002),
  add the wasm-runner compiled-module cache, and generalize ADR 0002 to a runtime-agnostic
  task-runtime boundary. This is a genuine *second task runtime*, hence its own phase.

Why this split: A is honest about what the code does today (the perf doc itself lists
"register it as a TaskDriver" as still-open), keeps ADR 0002 clean, and is one-night-sized.
The `TaskDriver` trait is two methods (`check`, `start`), so A→B is an additive wrapper over
A's `WasiRunner` — no rework of A's code.

See **"Phase A cycles"** and **"Phase B cycles"** below for the per-commit breakdown; the
numbered Steps 1–7 are the Phase-A implementation detail.

## Background: why not just merge

- Merge base: `e4dbcc2`. `rust` is +134 commits, `rs-speed` is +21.
- `rust` = deep structural refactor (68 `Split`, 18 `Cover` test, 11 `Extract`). No new
  features; big files broken into module trees.
- `rs-speed` = perf/feature spike: shared WASI linker crate, a compiled-wasm runner,
  module caching, 7 "new" WASI syscalls, benchmarks.
- 10 files changed on both branches. A blind merge collides hardest exactly where `rust`
  did its biggest deletions (`cli/lib.rs` −1478, `qjs/wasi_host.rs` −996, `host/state.rs` −217).

## The key reframe (drives the whole plan)

`rust`'s engine **already implements all 20 WASI syscalls** in its modular
`crates/wanix-qjs-engine/src/host/fs/*` tree — including the 7 that rs-speed "added"
(`fd_readdir`, `fd_tell`, `fd_filestat_set_size`, `path_rename`, `path_symlink`,
`path_readlink`, `path_remove_directory`). `rust`'s qjs WASI marshalling is *more*
complete than rs-speed's.

rs-speed's `wanix-wasi-host` shared linker + `WasiBacking::Ctx` existed to retrofit those
syscalls onto the **old monolith** that lacked them. `rust` took the better road (native
modular `host/fs`). Therefore:

- **Do NOT migrate qjs onto the shared linker.** `rust`'s qjs WASI path stays exactly as-is.
- **Do NOT port** rs-speed's engine-side changes: `WasiBacking::Ctx`, the `state.rs`
  `WasiHost` impl, `define_ctx_wasi_overrides`, the `qjs/wasi_host.rs` deletion. None are
  needed. This also erases the `delete-vs-split` conflict that made a merge ugly.
- **Bring `wanix-wasi-host` only as the wasm runner's marshalling layer** — standalone,
  generic `<S: WasiHost>`, zero engine coupling.

Net effect: the "hard" conflicts evaporate. What remains is mostly additive new crates +
one small memfs gap + a module cache + a new CLI subcommand + benchmarks.

## Per-feature verdict

| Feature | Verdict | Notes |
|---|---|---|
| qjs WASI marshalling | Keep `rust`. Port nothing. | rust already has all 20 syscalls, tested. |
| memfs `symlink`/`read_link` | **Port from rs-speed** | The one real backing gap. Also upgrades rust's qjs symlink-on-memfs. |
| `wanix-wasi-host` crate | **Port, wasm-only scope** | Standalone; all WasiCtx methods it needs exist on rust. |
| `wanix-wasm` runner + fixture | **Port clean** | New crate, no conflict. |
| module cache | **Port clean** | Genuine new win; rust does not cache compiled wasm today. |
| CLI | **Add `wasm` subcommand only** | Don't port rs-speed's flag refactor — rust already extracted it better. |
| benchmarks + docs | **Port, adapt** | No API drift; only CLI streaming→collected adaptation. |
| `MSG=hello`, `MSG=world`, `fixtures/out.txt` | **Skip** | Stray test droppings. |

---

## Port steps (dependency-ordered)

### Step 1 — memfs `symlink` + `read_link`

**Why first:** unblocks symlink syscalls for both the wasm runner and rust's own qjs path
(today rust registers `path_symlink` but memfs falls through to the `FsTrait` default
`Err(NotSupported)`; only `localfs` works).

**Investigation result:** rust's `Node` is structurally identical to rs-speed's
(`kind: FileType`, `data: Vec<u8>`, `FileType::Symlink` variant already exists in
`crates/wanix-fs/src/metadata.rs`). Dispatch chain already wired:
`WasiCtx::path_symlink` → `Namespace::symlink` → `filesystem.symlink`
(`crates/wanix-wasi/src/ctx/path_ops.rs`, `crates/wanix-vfs/src/lib.rs`). Adding the two
`MemFs` methods is sufficient — no changes to `WasiCtx`, `Namespace`, or `traits.rs`.

**Changes:**
- `crates/wanix-fs/src/memfs.rs`: add `read_link` and `symlink` to the `FileSystem` impl.
  Port rs-speed's bodies (rs-speed `memfs.rs:255-281`), adapting the lock access to rust's
  style: rs-speed uses `read_nodes()/write_nodes()` helpers; rust uses
  `self.nodes.read()/.write()` directly. Map poisoned-lock to rust's existing error variant.
  - `symlink`: reject `.`; reject existing path (`AlreadyExists`); validate parent is a
    directory; insert `Node::file(target.to_vec(), 0o777)` then set `kind = FileType::Symlink`.
  - `read_link`: look up node; `Err(InvalidPath)` if `kind != Symlink`; else clone `data`.
- `crates/wanix-fs/src/memfs/tests.rs`: add `#[test]`s in rust's inline style:
  - symlink create + `read_link` round-trip (target bytes preserved)
  - **lstat-style metadata: after `symlink`, `metadata` on the link path reports
    `FileType::Symlink` and length == target byte length** (proves the lstat half of the
    promised scope, not just create/readlink)
  - `read_link` on a non-symlink (regular file / dir) errors
  - symlink over an existing path errors (`AlreadyExists`)
  - symlink with a missing or non-directory parent errors

**Scope limit (don't overclaim).** This adds `symlink` / `read_link` / `lstat`-style
metadata on MemFs — it does **not** implement symlink *following* on `open` or on
`metadata_with_lookup(FollowSymlink)`. The default trait method ignores lookup mode
(`crates/wanix-fs/src/traits.rs:173`) and `MemFs::open` opens the node directly
(`crates/wanix-fs/src/memfs.rs:131`). rs-speed also stopped at create+readlink (no follow),
and the wasm guest fixture only exercises `--symlink`/`--readlink`, so non-following is
sufficient for parity. State the promise as "symlink/readlink/lstat on MemFs"; if follow
semantics are wanted later, that's a separate change with its own open/metadata tests.

**Effort:** small. No struct changes.

### Step 2 — `wanix-wasi-host` crate (wasm-scoped)

**Investigation result:** crate is already standalone — deps are only
`wanix-fs`, `wanix-wasi`, `wasmtime` (45.0.0, `default-features=false`, features
`anyhow`/`runtime`/`std`). No qjs/engine dependency. All 22 `WasiCtx` methods it calls
exist on `rust` with matching signatures (verified across `wanix-wasi` `fd_ops.rs`,
`fd_ops/stat.rs`, `path_ops.rs`, `open.rs`, `ctx.rs`). **No missing methods.**

**Changes:**
- Copy verbatim from rs-speed: `crates/wanix-wasi-host/Cargo.toml`, `src/lib.rs`, `src/fd.rs`,
  `src/path.rs`, `src/mem.rs`.
- Root `Cargo.toml`: add `"crates/wanix-wasi-host"` to `[workspace] members`.
- `justfile`: add `--package wanix-wasi-host` to the explicit `fmt` package list (line 4).
  The list is hard-coded, not workspace-wide, so a new crate is silently excluded from
  `just check`'s format gate otherwise. (Alternative: switch `fmt` to `cargo fmt --all`.)
- `Cargo.lock`: regenerate (`test` runs `--locked`); the crate must be clippy-clean
  (`clippy` is `--workspace --all-targets -D warnings`).
- Public surface kept: trait `WasiHost { wasi(&mut self) -> &mut WasiCtx; clock_time_ns(&self) -> u64; on_proc_exit(&mut self, code: i32); }`
  and `pub fn add_to_linker<S: WasiHost + 'static>(linker: &mut Linker<S>) -> Result<()>`.
  All three trait methods are needed by a standalone runner (ctx access, deterministic clock,
  exit-code capture) — nothing to trim.

**WASI surface caveat — the wasm runner is a first-pass command subset, not full Preview1.**
`add_to_linker` registers `poll_oneoff` as `ERRNO_NOSYS` (the qjs engine has a *real*
`poll_oneoff` path; the runner does not). So the runner supports command-style guests
(`_start`, fd/path I/O, args/env, clock, exit) with **no poll readiness**. This is fine for
the fixture and benchmarks, but the CLI `wasm` help text and docs should say so — it is not a
general-purpose WASI host. (This also reinforces "do not migrate qjs onto the shared linker":
the engine path is strictly richer.)

**Effort:** small (mostly copy). Build in isolation before Step 3.

> Note on the engine tech debt the prior investigation flagged (the `unreachable!()` in
> `HostState::wasi()`, the dead clock/random shadow, the one-shot exit hook): all of that
> lived in rs-speed's **engine** integration (`state.rs` / `host.rs`), which we are NOT
> porting. The standalone crate itself carries none of it.

### Step 3 — `wanix-wasm` runner + fixture

**Depends on:** Steps 1 (symlink) + 2 (linker).

**Investigation result:** public API is clean and dep-compatible.

```rust
pub struct WasiState { ctx: WasiCtx, clock_ns: u64, exit_code: Option<i32> }
impl WasiState { pub fn new(ctx, clock_ns) -> Self; pub fn exit_code(&self) -> Option<i32> }
// impl wanix_wasi_host::WasiHost for WasiState { ... }

pub struct WasiRunner { engine: Engine, module: Module }
impl WasiRunner {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self>;        // compiles via wasmtime
    pub fn run(&self, config: WasiConfig) -> Result<i32>;   // runs _start, returns exit code
    pub fn run_in_dir<S: AsRef<str>>(&self, dir, argv: &[S]) -> Result<i32>;
}

pub struct CaptureFile { /* Arc<Mutex<Vec<u8>>> */ }       // impl File
impl CaptureFile { pub fn new(); pub fn contents() -> String; pub fn bytes() -> Vec<u8> }
pub fn host_stdout() -> Box<dyn File>;
pub fn host_stderr() -> Box<dyn File>;
```

**Changes:**
- Copy `crates/wanix-wasm/` from rs-speed: `Cargo.toml`, `src/lib.rs`, `fixtures/` (incl.
  the committed `rust-guest.wasm` and `fixtures/guest-src/` with its own `Cargo.toml`/`src/main.rs`),
  `fixtures/.gitignore`. **Skip** `fixtures/out.txt` (stray output).
- Deps: `anyhow`, `wanix-fs`, `wanix-vfs`, `wanix-wasi`, `wanix-wasi-host`, `wasmtime` 45
  (features `anyhow`/`cranelift`/`runtime`/`std`). All present on rust.
- Root `Cargo.toml`: add `"crates/wanix-wasm"` to members.
- `justfile`: add `--package wanix-wasm` to the `fmt` list (same reason as Step 2).
- `tools/module-line-baseline.txt`: `just module-lines` (`tools/check-module-lines.sh`) enforces
  per-module line caps. `wanix-wasm/src/lib.rs` is ~599 lines — check it against the cap and
  either split it (rust-branch style) or add the baseline entry. rs-speed edited this file; do
  the rust-appropriate thing rather than copying rs-speed's baseline blindly.
- Keep the inline tests (VFS sharing, symlink round-trip, readdir/readlink/truncate error paths).
- Guest fixture rebuild command (document in crate README/comment, already used by rs-speed):
  `cargo build --release --target wasm32-wasip1` in `fixtures/guest-src/`, output committed as
  `rust-guest.wasm`. Guest modes: `--list/--rename/--rmdir/--truncate/--symlink/--readlink/--tell/--pi/--utime/--echo`.

**Effort:** medium (it's the biggest new surface, but conflict-free).

### Step 4 — module cache

**Investigation result:** copy-level, **no new deps** (`sha2` already in rust's
`wanix-qjs-engine/Cargo.toml`). Cache stores wasmtime-serialized `.cwasm`, keyed by
`sha256(wasm-bytes)`; wasmtime's embedded version marker makes stale artifacts auto-recompile.
Storage dir from `WANIX_QJS_CACHE_DIR` else `std::env::temp_dir()/wanix-qjs-module-cache`.
Atomic write via pid-suffixed temp + rename. All cache I/O is advisory (failures fall back
to fresh compile).

**Changes:**
- New file `crates/wanix-qjs-engine/src/module/cache.rs` (copy rs-speed, 82 lines). Fits
  rust's existing `module/` submodule layout (`module/abi.rs` already there). Exposes
  `pub(super) fn load_or_compile(engine, bytes, wasm_sha256: &[u8;32], cache_dir: &Path) -> Result<Module>`.
- `crates/wanix-qjs-engine/src/module.rs`:
  - Add `mod cache;`.
  - Refactor existing `from_bytes` (rust `module.rs:85-92`) — extract the ABI-validate +
    sha256 tail into a `fn finish(module, bytes) -> Result<Self>` helper.
  - Add `pub fn from_bytes_cached(engine, bytes, cache_dir: &Path) -> Result<Self>` (uses
    `cache::load_or_compile` then `finish`) and
    `pub fn from_bytes_with_default_engine_cached(bytes, cache_dir) -> Result<Self>`.
- New file `crates/wanix-qjs/src/bundled.rs` (copy rs-speed, ~15 lines):
  `pub fn bundled_module_cache_dir() -> PathBuf`.
- `crates/wanix-qjs/src/lib.rs`: add `mod bundled;`, `pub use bundled::bundled_module_cache_dir;`,
  and change `QuickJsRunner::from_bundled_wasm()` (rust `lib.rs:110-111`) to call
  `QuickJsModule::from_bytes_with_default_engine_cached(QUICKJS_WASM_FIXTURE, &bundled_module_cache_dir())`.

**DECISION — this is now an intentional runtime decision (resolves the AGENTS.md follow-up).**
The AGENTS.md Queued Follow-up said "keep production runner caching out of scope unless it
becomes an intentional runtime decision." We are making that decision: ship the cache, record
it in an ADR. Rationale (from `performance.md`, the source of these numbers): cold-start of a
qjs CLI invocation is ~545 ms and **~100% of it is Wasmtime cranelift-compiling** the 1.7 MiB
QuickJS wasm; the JS work is noise. Caching the serialized module makes that ~0.5 ms
(**~1000×**) and cuts peak RSS ~5× (77→15 MiB). End-to-end warm `qjs <file>` drops ~545 ms →
~18 ms (now dominated by OS process spawn, not the stack).

ADR task (part of this step): extend **ADR 0002** (or add a small new ADR) to record the
bundled-module cache as an accepted runtime decision — the cache key (sha256 of wasm), the
advisory/recompile-on-mismatch contract, the cache-dir trust boundary (below), and the
explicit non-goal (this is the *bundled qjs* module cache; a wasm-runner cache is Phase B).
Then remove the now-resolved line from AGENTS.md Queued Follow-ups.

**Payoff:** bundled-qjs cold start ~545 ms Cranelift compile → ~0.5 ms deserialize on warm cache.

**SECURITY — trust boundary (must resolve before landing).** The cache stores Wasmtime
serialized artifacts and loads them via `Module::deserialize`, which is **`unsafe`**:
deserializing attacker-controlled bytes is arbitrary-code-execution-equivalent. rs-speed's
default path is `std::env::temp_dir()/wanix-qjs-module-cache` — a shared, world-writable,
predictable location on multi-user hosts. Treating those bytes as merely "advisory" is not
enough; a local attacker who pre-seeds a `<sha256>.cwasm` file gets code execution in the
victim's process. The sha256 key authenticates the *input wasm*, not the *cached artifact*.
Required hardening for the port (pick a coherent subset):
- Default to a **per-user, non-world-writable** dir (e.g. under the user cache dir with
  `0700`), not shared `temp_dir()`; create it with owner-only perms and verify ownership +
  perms on read.
- Only honor `WANIX_QJS_CACHE_DIR` when it points at an owner-safe dir (or treat the env var
  as explicit operator opt-in to a trusted path, documented as such).
- Optionally gate cache reads behind an integrity check (e.g. store under a dir keyed to the
  effective UID; reject artifacts not owned by the current user).
Keep the advisory-fallback behavior (miss/parse-fail → fresh compile) — but make a *hostile*
cache entry non-loadable, not just a corrupt one. Document the chosen model in `cache.rs`.

**Effort:** small (port) + small (the hardening above — do not skip it).

### Step 5 — CLI `wasm` subcommand

**Investigation result:** **Don't** port rs-speed's `try_parse_common_flag` refactor — rust
already extracted common flag parsing better (`parse_common_qjs_option` in
`qjs_args/common.rs`). Also, rust uses a **collected** handler model
(`fn(&[OsString], &mut dyn Read) -> Result<CliOutput, CliError>`), whereas rs-speed's
`wasm_run.rs` used a **streaming** handler. The wasm command must be reshaped to rust's
collected convention (capture output into `CaptureFile`, return `CliOutput`).

**Changes (follow the qjs pattern exactly):**
- New `crates/wanix-cli/src/wasm.rs`:
  `pub(super) fn run_wasm(command: WasmCommand, process_stdin: &mut dyn Read) -> Result<CliOutput, CliError>`
  — builds a `Namespace`/`WasiConfig`, runs `WasiRunner::from_bytes(..).run(..)` with
  `CaptureFile` sinks, returns a `CliOutput` (mirror `qjs.rs`).
- New `crates/wanix-cli/src/wasm_args/mod.rs` (+ `command.rs` if parsing is non-trivial):
  `WasmCommand { path, args, env, cwd, stdin }` and
  `pub(crate) fn parse_wasm_command(args: &[OsString]) -> Result<WasmCommand, CliError>`,
  with inline `#[cfg(test)]` tests matching the "Cover" convention.

  **Parser shape — do NOT reuse `parse_common_qjs_option` directly.** That parser is backed
  by `QjsRunOptions` (`qjs_args/common.rs`), which bundles env/cwd/stdin **together with
  qjs-only flags**: `--event-loop-ms`, `--ready-io-turns`, `--interrupt-after`,
  `--memory-limit`, `--mount`. Reusing it would make `wanix wasm` silently accept flags that
  mean nothing to a wasm command. Instead, build a small wasm-specific options struct that
  handles only `--env`, `--cwd`, `--stdin`, `--stdin-file`, composed from the **lower-level
  helpers** the qjs setters already call: `validate_env_line`, `set_qjs_stdin`,
  `NormalizedPath::new`, `os_arg_to_string`. (If a genuinely shared invocation parser for just
  those four flags is worth it, extract one and have both qjs and wasm use it — but that's a
  refactor, not a reuse; size it before committing.)
- `crates/wanix-cli/src/collected.rs`: add `("wasm", run_wasm_collected)` to
  `COLLECTED_COMMANDS` and a `fn run_wasm_collected(rest, stdin)` that calls
  `parse_wasm_command` then `run_wasm`.
- `crates/wanix-cli/src/help.rs`: add the `wasm` usage line, matching the existing
  `wanix-rust <cmd>` convention in `USAGE` (NOT `wanix`):
  `wanix-rust wasm [--env KEY=VALUE ...] [--cwd DIR] [--stdin ...] FILE.wasm [args...]`.
  Note the WASI subset (command-style, no poll readiness) so users don't expect full Preview1.
- `crates/wanix-cli/Cargo.toml`: add `wanix-wasm` dependency (and a dev-dep for the bench
  examples in Step 6).

**Effort:** small–medium (the streaming→collected reshape is the only real adaptation).

### Step 6 — benchmarks + docs

**Investigation result:** **no API drift** — every runner method the benches call exists on
rust (`WasiRunner::from_bytes/run`, `WasiConfig` builder, `CaptureFile`,
`QuickJsRunner::from_bundled_wasm`, `run_source_with_wanix_config`, and the engine/wasmtime
APIs used by the qjs-engine examples).

**Changes:**
- Copy `crates/wanix-cli/examples/compute_bench.rs`, `examples/shared_vfs.rs`.
- Copy `crates/wanix-qjs-engine/examples/{startup_bench,concurrency_bench,wasm_speed_bench}.rs`.
- Copy `crates/wanix-cli/tests/shared_vfs_differential.rs` (the qjs↔wasm VFS parity proof).
- Copy `performance.md`, `docs/scaling-eli5.md`, and the `AGENTS.md` additions.
- Add the `[[example]]`/dev-dep wiring each bench needs in the relevant `Cargo.toml`
  (`wanix-cli` gains a dev-dep on `wanix-wasm`).

**Effort:** small.

### Step 7 — verify

- Focused first: `cargo test -p wanix-fs` (Step 1), build each new crate in isolation
  (Steps 2–3), then the differential test
  `cargo test -p wanix-cli --test shared_vfs_differential` (proves qjs and the wasm runner
  share one `Namespace`/memfs both ways).
- Smoke the new CLI command end-to-end against `rust-guest.wasm`.
- Optionally run `compute_bench` / `shared_vfs` examples to confirm the perf tiers work.
- **Final gate: `just check`** (repo guardrail = `fmt` + `module-lines` + `clippy -D warnings`
  + `test --locked`). This is the real bar, not bare `cargo build`/`cargo test`. Confirm the
  `justfile` `fmt` list and `tools/module-line-baseline.txt` were updated in Steps 2–3 so this
  passes.

---

## Phase A cycles (commit-per-cycle; each ends green)

One commit per cycle (Cycle Rules: commit each completed cycle). `just check` must pass before
every commit. Ordered by dependency. Steps 1–7 above are the implementation detail; the cycles
group them into committable units.

- **A1 — memfs symlink/readlink/lstat** (Step 1). `cargo test -p wanix-fs` green. Commit.
- **A2 — `wanix-wasi-host` crate** (Step 2). New crate builds; workspace member + `justfile`
  fmt list + `Cargo.lock` updated; clippy-clean. Commit.
- **A3 — `wanix-wasm` runner + fixture** (Step 3). Depends on A1+A2. **Split `lib.rs` to ≤350
  non-test lines** (it's ~599; module-line hard limit is 350) — this is real work, not a copy.
  Inline tests green. Commit.
- **A4 — module cache + ADR** (Step 4). Cache + owner-safe-dir hardening + ADR 0002 update +
  remove the resolved AGENTS.md follow-up line. `cargo test -p wanix-qjs-engine -p wanix-qjs`
  green. Commit.
- **A5 — CLI `wasm` subcommand** (Step 5). Depends on A3. Collected-model handler, wasm-specific
  parser, `wanix-rust wasm` help line noting Tier-1/not-a-driver. Parser boundary tests. Commit.
- **A6 — benchmarks + docs + differential test + capability map** (Step 6). Depends on A3+A5.
  Add the capability-map line in `AGENTS.md` framing wasm as a Tier-1 demo runner (explicitly
  *not* a task driver). `shared_vfs_differential` green. Commit.
- **A-gate — review/cleanup pass.** Per Cycle Rules ("every ~5 feature commits, run a
  review/cleanup pass"): full `just check`, a review of the accumulated A1–A6 diff (correctness
  + the cache trust boundary + module-line compliance + ADR accuracy), fix findings, final
  commit. **Phase A done = all green, committed, reviewed.**

## Phase B cycles (only after Phase A is fully done)

Turns the Tier-1 runner into a first-class Wanix task runtime. Additive over A's `WasiRunner`.

- **B1 — `WasmTaskDriver: TaskDriver`.** `check()` matches `.wasm`; `start()` runs the existing
  `WasiRunner` against the Task's Wanix state (namespace, env/cwd/argv, fd 0/1/2 from the task
  fd table, exit status observable through task state). Register in the driver registry.
- **B2 — fd mirroring (ADR 0002 contract).** When the wasm guest exposes a Wanix-observable fd,
  mirror it through the task fd table and release on guest close (same rule qjs follows).
- **B3 — `#task/new/wasm` creation + auto-start.** A `.wasm` cmd allocates task identity +
  service files and auto-starts via `check()`; observable exit; shared VFS proven at the
  *task* level (not just the bare runner).
- **B4 — wasm-runner module cache.** Mirror `from_bytes_cached` for the wasm runner (the
  `performance.md` follow-up); reuse the same owner-safe cache-dir model from A4.
- **B5 — ADR generalization.** Update ADR 0002 (or successor) so the task-runtime boundary is
  runtime-agnostic: Wanix owns task state + live WASI for *any* WASI task runtime (qjs and
  wasm); the runtime crate owns engine mechanics. Update the capability map to promote wasm
  from "Tier-1 demo runner" to "Tier-1 task driver."
- **B-gate — review/cleanup pass + `just check`.** Same bar as A-gate.

## Execution model

The **edits are sequential, not parallel-agent**: this is gated, must-compile code where
ordering matters (A1→A3→A5) and `just check` is the per-commit bar. Driving it as a background
multi-agent Workflow that edits crates in parallel would race the build and be hard to review
mid-run. So: I implement cycle by cycle, run focused tests + `just check`, commit, move on.

Where fan-out *is* the right tool: the **A-gate and B-gate review passes** — a parallel
adversarial review of the accumulated diff (correctness lens, trust-boundary lens, module-line
/ guardrail lens, ADR-accuracy lens) before the final commit. Those gates can be a Workflow;
the implementation cycles are sequential.

---

## Explicitly NOT porting

- `crates/wanix-qjs/src/wasi_host.rs` deletion (rust keeps + has already modularized it).
- `WasiBacking::Ctx` and the engine `state.rs` `WasiHost` impl / `define_ctx_wasi_overrides`
  (qjs is not migrated onto the shared linker).
- rs-speed's `try_parse_common_flag` CLI refactor (rust's extraction is better).
- `MSG=hello`, `MSG=world`, `crates/wanix-wasm/fixtures/out.txt` (junk).

## Deferred follow-up (after this lands)

`rust`'s `host/fs/layout.rs` and `wanix-wasi-host/mem.rs` duplicate the WASI Preview1 wire
format (errno→preview1 codes, dirent/filestat/prestat byte layout, iov reads). The clean
end-state is to factor those primitives into `wanix-wasi-host` as the single source and have
both the qjs engine's `host/fs` and the wasm runner build on them. Principled but invasive —
deliberately deferred so it doesn't re-disturb rust's 134-commit cleanup during the port.

## Risk / effort summary

| Step | Effort | Conflict risk | Notes |
|---|---|---|---|
| 1 memfs symlink | S | none | struct compatible; scope = symlink/readlink/lstat, no follow |
| 2 wanix-wasi-host | S | none | standalone; +justfile fmt; poll_oneoff = NOSYS (subset) |
| 3 wanix-wasm | M | none | biggest new surface; +justfile fmt, module-line baseline |
| 4 module cache | S + **security** | none | no new deps, but **trust-boundary hardening required** before landing |
| 5 CLI wasm cmd | S–M | low | streaming→collected reshape; wasm-specific parser (NOT QjsRunOptions) |
| 6 benchmarks/docs | S | none | no API drift; cli dev-dep on wanix-wasm |
| 7 verify | — | — | `just check` is the gate, not bare cargo |

## Changelog (post-review)

Revised after Codex review of the first draft:
- **[P1]** Step 4: added a mandatory module-cache trust-boundary section (`unsafe
  Module::deserialize` over a world-writable temp path → per-user owner-safe dir + perm checks).
- **[P2]** Step 2: documented the wasm runner's WASI subset (`poll_oneoff` = NOSYS, no poll
  readiness) — it is not a full Preview1 host.
- **[P2]** Step 5: replaced "reuse rust's common-flag parser" with an explicit wasm-specific
  parser built on the lower-level helpers; `QjsRunOptions` would accept qjs-only flags.
- **[P2]** Steps 2–3, 7: added `justfile` `fmt`-list updates, `Cargo.lock` `--locked`, and
  `tools/module-line-baseline.txt` as concrete tasks; `just check` is the final gate.
- **[P3]** Step 1: narrowed the MemFs symlink promise to create/readlink/lstat (no follow-on-open).

Second review pass:
- **[P3]** Step 5: help text uses `wanix-rust wasm` (the actual `USAGE` prefix), not `wanix wasm`.
- **[P3]** Step 1: added an lstat-style test (symlink `metadata` reports `FileType::Symlink` +
  target length) so the claimed scope is proven, not just create/readlink.

Scoping decisions (post AGENTS.md / ADR review):
- **Module cache: IN, as an intentional ADR-backed decision** (user call). Justified by
  `performance.md` (~1000× cold-start, ~5× RSS). A4 includes the ADR 0002 update + removing the
  resolved AGENTS.md follow-up + owner-safe cache-dir hardening.
- **wasm runner: Phase A = Tier-1 demo/bench harness, NOT a task driver** — matches what the
  code does and the perf doc's own "still open" note; keeps ADR 0002 clean.
- **Phased: A (full Tier-1 integration, green+committed+reviewed) THEN B (`.wasm` as a real
  `#task/new/wasm` driver + ADR generalization).** B is additive over A's `WasiRunner`.
- **Execution: sequential commit-per-cycle with `just check` gate;** Workflow fan-out reserved
  for the A-gate/B-gate adversarial review passes only.
