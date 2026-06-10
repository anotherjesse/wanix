# Rust Wanix Focus Report

Date: 2026-06-06

This report synthesizes nine subagent research passes plus local inspection of
the Rust Wanix workspace and the COZ Codex app-server worktree. It is a product
and architecture focus note, not an ADR. The north star remains the existing
one: a Rust-native Wanix core outside Chrome, with Wasmtime as the execution
substrate and QuickJS/WASI as the first serious task runtime.

## Executive Take

Wanix already has enough substrate to become useful: shared VFS, qjs and wasm
task drivers, `#task`, `#term`, 9P, served workbench, qjs terminals, and
QEMU/v86 handoffs. The next high-leverage work is not another hidden runtime
layer. It is making Wanix feel like a small programmable OS.

The strongest near-term direction is:

1. Make writing Wanix programs pleasant.
2. Make the served/editor/web surfaces run those programs.
3. Add agent, database, collaboration, and trace capabilities as Wanix services
   exposed through files and tasks.

The most important design habit is to keep new power behind host-side service
files and explicit task capabilities. qjs and wasm programs should interact
through WASI, VFS paths, stdio, env, and service files, not runtime-specific
globals.

## Recommended Bets

### 1. Developer Kit And Userland

Build the first real inside-Wanix userland:

- `/bin` commands implemented as qjs or wasm programs.
- `/lib/wanix` helper modules for service-file programming.
- project templates for qjs, typed qjs, and Rust wasm.
- PATH-based command execution in `qjs-shell`.
- a small `wanix test` harness for qjs/wasm programs.

Why this first: every other direction gets easier when users can write and run
programs without hand-wiring service files in each example.

First milestone:

- `wanix new qjs|qjs-ts|rust-wasm`
- `/lib/wanix/{fs,task,process}.js`
- matching `.d.ts` declarations
- `/bin/help`, `/bin/tree`, `/bin/grep`, `/bin/taskctl`
- one qjs template test and one Rust wasm template test

### 2. TypeScript And Type Checking

Treat TypeScript as a build-time layer, not as something QuickJS needs to
understand directly.

Useful pieces:

- `wanix-qjs.d.ts` for `qjs:std`, `qjs:os`, `scriptArgs`, `print`, `console`,
  and the Wanix helper modules.
- `tsconfig.wanix-qjs.json`.
- `wanix qjs-check` or `wanix qjs --check`.
- optional `qjs-ts` command that transpiles outside the runtime, then runs
  plain JavaScript in qjs.
- workbench diagnostics for `wanix:` qjs files.

Guardrail: do not imply Node compatibility inside QuickJS. Build with Node or
TypeScript tooling outside Wanix when needed, then run plain JS inside Wanix.

### 3. Webby HTTP Programs

Add a command-style HTTP task runner to `serve`.

Shape:

- `wanix.toml` or `.wanix/http.json` declares routes, task kind, entrypoint,
  cwd, env, body limit, and storage.
- an HTTP request starts a qjs or wasm task.
- request metadata arrives through env and/or stdin.
- stdout becomes the response body.
- stderr and task status show up in discovery/workbench.

First milestone: loopback-only HTTP programs under `serve --wanix-services`,
with GET/POST, a body cap, and one qjs demo. This is the simplest path toward
"Cloudflare Workers, but Wanix-y" without inventing a new web framework.

### 4. Workbench / VS Code Integration

The workbench already has the right bones: it can browse `wanix:/`, run the
active JS file as qjs, and open qjs-backed terminals. Turn it into the main
daily-driver surface.

Focus:

- run active `.js` and `.wasm` files.
- show `#task` status and exits.
- preview HTTP programs.
- keep terminal lifecycle/disposal reliable.
- add remote file watch/event support later.
- advertise all actual task drivers in discovery.

Concrete cleanup: `serve_task_table` registers `noop`, `qjs`, and `wasm`, but
service discovery currently advertises only `noop` and `qjs`.

### 5. Agentic Layer Through Codex App-Server

The COZ app-server branch is the right model to study. It lives at:

```text
/Users/jesse/.codex/worktrees/3583/coz
branch: codex/app-server-coz-exec
```

COZ starts `codex app-server --listen stdio://`, builds a `CODEX_HOME` overlay,
writes an environment config, and exposes a COZ exec server with process and
filesystem JSON-RPC methods.

For Wanix, copy the shape but back it with Wanix:

- host-side `AgentEngine` trait with a fake backend for tests.
- optional `CodexAppServerAgentEngine` backend.
- a Wanix exec environment backed by VFS, `#task`, and `#term`.
- `#agent` service files for sessions, prompts, replies, transcripts, tool
  events, and control.
- maybe `#task/new/agent` for one-shot agent tasks.
- qjs-shell command: `agent "explain this file"` or `agent fix foo.js`.

First milestone: one-shot read-only agent task using a fake engine in tests and
Codex app-server as an optional host integration. Make visible tool events part
of the contract from day one.

### 6. Multi-User And Collaboration

Start with identity and authority, not collaborative editing.

Focus:

- `--auth-token` for non-loopback serve.
- discovery metadata describing auth policy.
- `SessionPrincipal` at the serve boundary.
- terminal ownership and read-only attach.
- `#session` or `#presence`.
- filesystem watch/events for editor clients.

This is a prerequisite for remote VS Code, shared terminals, Automerge sync,
and any public WebSocket route. It also protects the current host-root serving
model from accidentally becoming a remote write API.

### 7. Database And Automerge Services

Add storage as host-side Wanix services, not qjs globals.

Good first shape:

- new `wanix-doc` crate depending on `wanix-fs` plus Automerge.
- in-memory `DocStore`.
- `#doc/new`
- `#doc/<id>/json`
- `#doc/<id>/heads`
- `#doc/<id>/ctl`
- bind `#doc` in `serve --wanix-services`.
- advertise it in discovery.

Later:

- `#store` object/CAS store.
- redb or SQLite-backed durable filesystem.
- Automerge VFS projection.
- loopback-only Automerge sync route.
- qjs userland tools: `docctl`, `dbctl`, `watchdoc`.

Guardrail: keep large blobs out of Automerge documents. Use a blob/object store
and reference content hashes from documents.

### 8. Trace, Replay, And Capability Manifests

Wildcard idea worth taking seriously: Wanix Black Box.

Build a recorder that captures:

- task graph
- cwd/env/cmd
- stdout/stderr
- terminal bytes
- VFS mutation summaries
- final filesystem digest

Then add `wanix replay` to rerun and assert output/digest. This would make
demos, agent audit trails, debugging, and runtime compatibility work much
sharper.

Related idea: per-task capability manifests that mount only the paths and
devices a task needs: `/src`, `/tmp`, `#term`, `#task`, future `#net`, future
`#doc`.

### 9. Code Quality / Architecture

Production module size is currently healthy: `just module-lines` passes and the
baseline file has no active oversized entries. The current pressure is test
mass and contract drift.

Good cleanup pass:

- move the giant internal test block out of `crates/wanix-cli/src/lib.rs`.
- add a warning-only test-line report.
- fix the service discovery task-driver mismatch.
- replace hand-built JSON strings in discovery/handoff code with typed builders
  or serde-backed structures.
- normalize workspace metadata/lints for `wanix-wasi-host` and `wanix-wasm`.
- add a dependency-direction check to `just check`.
- refresh `AGENTS.md` queued follow-ups, since old line-size targets are now
  less accurate.

## Suggested Roadmap

### Cycle 0: Hygiene

- Fix service discovery so it advertises `wasm`.
- Move large `wanix-cli` tests into internal test modules.
- Add a non-failing test-line report.
- Start typed JSON builders for discovery/handoffs.
- Run `just check`.

### Cycle 1: Wanix Developer Kit v0

- Add templates.
- Add `/lib/wanix` qjs helpers and declarations.
- Add first `/bin` commands.
- Add PATH fallback in qjs-shell.
- Add template tests.

### Cycle 2: TypeScript And DX

- Add qjs type checking command.
- Add typed qjs starter.
- Add workbench diagnostics or at least generated config files.
- Add `docs/writing-wanix-programs.md`.

### Cycle 3: Web Programs

- Add loopback HTTP task runner.
- Add route manifest.
- Add qjs demo.
- Add workbench preview affordance.

### Cycle 4: Agent Service

- Add `AgentEngine` trait.
- Add fake backend.
- Add `#agent` or one-shot agent task.
- Add Codex app-server backend behind explicit host integration.
- Expose visible tool events.

### Cycle 5: Multi-User Floor

- Add auth token support.
- Add session principal.
- Add terminal ownership policy.
- Add discovery auth metadata.
- Add tests for accepted/rejected connections.

### Cycle 6: DocStore v0

- Add `wanix-doc`.
- Mount `#doc`.
- Add qjs demo and Rust sync test.
- Keep sync loopback-only until identity/auth are stronger.

### Cycle 7: Wanix Black Box

- Add trace/record hooks.
- Add filesystem digest.
- Add replay assertion.
- Use it to harden qjs/wasm/9P compatibility scenarios.

## Cross-Cutting Guardrails

- Keep lower-level crates free of runtime engines.
- Keep new capabilities exposed through VFS/service files.
- Do not reintroduce `globalThis.Wanix`.
- Do not hold namespace or filesystem locks while calling another filesystem or
  runtime engine.
- Keep public/network routes loopback-only until auth and session identity are
  explicit.
- Prefer qjs/wasm differential tests when adding user-facing behavior.
- Treat QEMU/v86 as handoff clients, not as Wanix-supervised VM lifecycle, until
  the contracts are stronger.

## Appendix A: COZ App-Server Notes

Relevant worktree:

```text
/Users/jesse/.codex/worktrees/3583/coz
branch: codex/app-server-coz-exec
```

Key COZ pieces:

- `src/chat/app_server.rs` defines a `ChatBackend` trait and a
  `CodexAppServerBackend`.
- It spawns `codex app-server --listen stdio://`.
- It creates a `.data/codex-home` overlay.
- It reuses auth/config by copying or linking from the real Codex home.
- It writes `environments.toml` with a default `coz` environment.
- It sends `thread/start` and `turn/start` JSON-RPC requests.
- It handles app-server notifications such as agent messages, command
  execution, and dynamic tool calls.
- `src/codex_exec_server.rs` implements process and filesystem JSON-RPC methods
  for the COZ environment.

Wanix adaptation:

- Replace the COZ bash/js environment with a Wanix environment.
- Implement process start/read/write/terminate through `#task` and `#term`.
- Implement fs read/write/list/metadata/copy/remove through Wanix VFS.
- Surface tool calls as visible event streams.
- Keep the app-server process host-side.
- Keep qjs guests interacting through files, stdio, env, and service files.

This suggests an incremental path:

1. Fake `AgentEngine` for deterministic tests.
2. Local one-shot agent task.
3. `#agent` service filesystem.
4. Codex app-server backend.
5. Workbench commands that call `#agent`.

## Appendix B: Perspective Details

### Agentic

Ideas:

- `#task/new/agent` one-shot driver.
- `#agent/new`, `<id>/prompt`, `<id>/reply`, `<id>/transcript`,
  `<id>/tools`, `<id>/ctl`.
- qjs-shell `agent` command.
- workbench commands for ask/explain/run.
- service discovery entries for agent capabilities.
- explicit tool permissions and visible tool events.

Risks:

- prompt injection through mounted files.
- unclear authority if agents can mutate host roots.
- app-server lifecycle becoming tangled with qjs runtime lifecycle.
- tests becoming nondeterministic without a fake backend.

### Webby

Ideas:

- route HTTP requests to qjs/wasm tasks.
- manifest-driven HTTP programs.
- `#http` service files.
- object-store integration for static/durable assets.
- workbench preview surface.
- eventually multiplexed authenticated session channels.

Risks:

- serving host roots remotely without auth.
- tasks blocking request threads.
- route shadowing with static files and well-known handoffs.
- promising Worker-like semantics before lifecycle/cancellation are ready.

### TypeScript

Ideas:

- declarations for qjs builtins and Wanix helper modules.
- `qjs-check`.
- `qjs-ts` compile-then-run flow.
- workbench diagnostics.
- typed runtime manifest.
- convert key demos to TS or `// @ts-check`.

Risks:

- implying Node module semantics.
- making npm/esbuild mandatory for normal Rust checks.
- hiding runtime boundaries behind bundler magic.

### Code Quality

Findings:

- `just module-lines` passes.
- production module size is currently in good shape.
- tests are the navigation hotspot.
- discovery advertises fewer task drivers than are registered.
- manual JSON construction is scattered through discovery/handoff code.
- lock boundaries are mostly healthy today.

Useful cleanup:

- split internal tests.
- add test-line reporting.
- fix discovery driver list.
- add typed JSON builders.
- add dependency-direction check.

### Build More Inside

Ideas:

- embedded userland image.
- `/bin` commands.
- PATH-based task launch.
- coreutils pack.
- service tools: `taskctl`, `termctl`, `mounts`.
- demos under `/demos`.
- tiny package/install story later.

Risks:

- continuing to grow the demo shell instead of moving commands into userland.
- lack of cancellation/job control for richer commands.
- package installs crossing host-mount trust boundaries.

### Multi-User / Collaboration

Ideas:

- token auth.
- `SessionPrincipal`.
- terminal ownership.
- `#presence`.
- role/policy wrapper filesystem.
- watch/events for remote editors.
- durable terminal backscroll.
- advisory locks before CRDT editing.

Risks:

- powerful service files exposed remotely.
- 9P identity fields being treated as auth.
- collaboration metadata living only in per-connection server state.

### DX

Ideas:

- templates.
- qjs SDK.
- qjs declarations.
- program manifests and `/bin`.
- `wanix test`.
- `wanix dev` watch loop.
- Rust wasm guest SDK.

Risks:

- overpromising hot reload.
- making external JS tooling mandatory.
- adding magical loaders that obscure what qjs actually supports.

### Database / Automerge

Ideas:

- `wanix-store` object store.
- SQLite/redb-backed durable FS.
- `#doc` Automerge service.
- Automerge VFS projection.
- hybrid CRDT plus blob store.
- loopback Automerge sync route.
- filesystem journal.

Risks:

- arbitrary host DB paths.
- whole-document writes through partial file writes.
- treating Automerge as auth/identity.
- putting large blobs directly in CRDT docs.

### Wildcard

Ideas:

- flight recorder / deterministic replay.
- capability manifests.
- copy-on-write roots.
- `#trace` observability device.
- portable sandbox appliance.
- filesystem time machine.
- VM specimen museum.
- runtime compatibility arena.
- `#net/tcp`.
- `#pipe` and `#watch`.

Most promising first milestone:

- Wanix Black Box v0: record one qjs program run, write JSONL, replay, and
  assert output plus filesystem digest.

## Appendix C: Immediate Small Fixes

These are small, high-signal items that fell out of the research:

- Update service discovery so registered task drivers and advertised task
  drivers agree.
- Add `wasm` support to workbench "run active file" where practical.
- Move qjs service-file helper code out of examples into `/lib/wanix`.
- Add a tiny `wanix-qjs.d.ts` before building a full TypeScript story.
- Keep `serve` remote routes loopback-only unless auth is explicitly enabled.
- Update `AGENTS.md` queued cleanup notes after the next cleanup pass.

