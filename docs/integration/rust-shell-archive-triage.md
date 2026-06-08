# Triage: the `rust` branch shell-archive work — port, re-implement, or drop

Purpose: decide what (if anything) to bring from the `origin/rust` branch into
`cockpit-mesh-integration`, and stage *when* to care. This is a decision record,
not a plan to execute now.

## TL;DR

The only thing on `rust` that `cockpit-mesh-integration` does **not** already
have (in better, mesh-aware form) is one self-contained feature theme: a
**qjs-shell audit/replay layer** — command history, per-command mutation
tracking, session archives, dossiers, and a retry queue.

Decision: **don't port it now.** It is single-node, host-directory-only, and
structurally cannot follow us onto the mesh. We lose only a browser-cockpit
shell-audit nicety that mostly serves agents. If shell history matters later,
**re-implement it mesh-native** rather than porting this code. The one cheap,
low-risk slice worth porting *if* we want something soon is the Rust-side
command-history core.

## How the two branches relate (so we don't re-litigate this)

- Shared content base: cockpit forked from `rust @ 7359eec`
  ("Confirm qjs shell mutations in workbench", 2026-06-06 22:22).
- `rust` was later rebased, so the git common ancestor *appears* as the older
  `25ddf4b`. By **content** the fork is `7359eec`; by **SHA graph** git reports
  a 118/89 two-way split. Trust the content view.
- Work done on `rust` *after* the fork = exactly **31 commits**, and they are
  **all one subsystem** (the shell-archive theme below). Everything else `rust`
  has (HTTP apps, duet, service inspector, system sidebar, self-check, agent
  repair demo, qjs/wasm starters) was already in cockpit's content base and was
  re-implemented on the mesh backend (the cockpit `restore` commits).
- A git merge / cherry-pick is **not** viable: `extension.ts` diverged ~5k
  lines (rust = monolith, cockpit = modularized), and the entire mesh crate set
  (`wanix-mesh`, `-mesh-wire`, `-kv`, `-pipe`, `-plumb`, `-cas`, `-id`, `-cpu`,
  `-agent`, `-9p-client`, `-sh`) does not exist on `rust`. Integration means
  re-porting selected features, not merging history.

## What the 31 commits actually are

One feature theme in three layers, all riding the **host-dir websocket
terminal** (`serve --root DIR` → `serve_terminal_websocket_connection(&roots.static_root, …)`):

1. **Observability** — shell mutation outcomes, "no-change" diagnostics,
   command-record evidence, structured agent-repair report.
2. **History** — persist command history to `.wanix/qjs-shell/commands.jsonl`
   (+ `latest.json` / `latest.md`), search, summary report, compaction.
3. **Archives** — export / compare / restore snapshots, retention, bundle
   import/export, explorer, health badges, and **dossiers** (runnable actions,
   action results/status, retry queue + batch).

Code surface:

- Rust (`crates/wanix-cli/src/serve/terminal_ws/`): `command_record.rs`,
  `shell_observation.rs`, `shell_activity.rs`, `shell_history.rs`,
  `root_changes.rs`, `protocol.rs` (cockpit's `terminal_ws` has only
  `message` / `request` / `session`).
- TS: `workbench/src/web/extension.ts` (+5097 lines on rust), `system-view.ts`
  (+588), `agent-tool-contract.ts` (+27).
- 31 screenshots + a 536-line DX doc.

## Why it dead-ends off the mesh

The substrate is identical on both branches — the qjs-shell websocket terminal
runs against `roots.static_root`, a real host directory. So the design does not
*conflict* with the mesh; it just cannot *grow* into it:

- `RootChangeTracker` detects mutations by polling `fs::read_dir` +
  `symlink_metadata` over the host tree. Blind to device writes
  (`#kv` / `#cas` / `#agent`), MemFs-only worlds, and `/n/<peer>` imports — none
  of those are host files.
- `ShellHistoryWriter` writes archives straight to host disk via `std::fs`,
  bypassing the `FileSystem` trait. Works only because `static_root` happens to
  be a host dir; on a `--wanix-services` composed namespace it writes to the
  wrong place and misses all device + mesh activity.

A mesh-native version would write through the served `Namespace` and replace
`fs` polling with namespace/device change signals. That is a rewrite of both the
writer and the tracker — not a port.

## Decision table

| Item | Verdict | Why |
|---|---|---|
| Browser cockpit features (HTTP apps, duet, agent repair, service inspector, system sidebar, self-check, starters) | **Already have** | Re-implemented on the mesh backend (cockpit `restore` commits). No action. |
| Shell command-history core (Rust `command_record` + `shell_observation` + `shell_history`, writing `commands.jsonl` / `latest.*`) | **Port only if we want shell audit soon; otherwise defer** | Cheap, low-risk on the Rust side (substrate identical on cockpit). But host-dir-only — see below. |
| `RootChangeTracker` (per-command mutation detection) | **Re-implement mesh-native, don't port** | `fs::read_dir` polling can't see devices/mesh. Replace with `Namespace`/device change signals. |
| Session archives / dossiers / retry queue | **Drop** | Demo-grade observability polish on top of the host-dir design; highest cost (most of the +5097 TS), least durable. |
| Structured agent-repair report | **Re-visit via `#agent`** | Cockpit's `#agent` already exposes `events` / `status` / transcript files. If we want a richer report, source it from the device, not from a host-dir shell log. |
| TS archive/dossier UI (`extension.ts` +5097, `system-view.ts` +588) | **Don't port** | Cockpit's `extension.ts` is modularized and diverged ~5k lines; the UI is the expensive half and it backs the dropped/re-implement items. |
| Core-crate refactors ("Split fs file handle", "WASI ctx fallible", "Split qjs event-loop", "9P frame size explicit") | **Skip unless a bug bites** | Refactors, not features; cockpit evolved these files independently for the mesh. Re-applying = churn. |
| `v86-shared-demo.ts` (separate from the 31; pre-fork) | **Port (already on the TODO)** | Cockpit has it as a `// STUB:`. Lift rust's impl instead of writing fresh. Tracked in CLAUDE.md follow-ups, not part of this triage. |

## When to care (staging triggers)

Care about shell audit/history **only when one of these is true**:

1. **Agents drive the shell unattended and we need a replayable trail** that the
   `#agent` device's `events`/transcript doesn't already cover.
2. **Users ask for shell history across cockpit reloads** as a real workflow
   need (not a demo).
3. **The interactive shell + per-task-thread work lands** (the queued
   `wanix-sh` pipeline-concurrency / interactive-input item). At that point the
   shell session model changes anyway, so build audit on top of the new model.

If/when triggered, the order is: (a) Rust command-history core, written
**through the `FileSystem`/`Namespace`** so it works for MemFs, devices, and
mesh imports — not `std::fs` on a `PathBuf`; (b) change detection from
namespace/device signals; (c) UI last, and only the slice users ask for.

## If we do port the cheap Rust slice anyway

For future reference — the command-history core ports cleanly because the
terminal substrate is identical on cockpit:

- Re-add `terminal_ws/{command_record, shell_observation, shell_history}.rs`.
- Port the `QjsShellSession::start_in_cwd_with_command_records` variant in
  `qjs_term.rs` (cockpit currently calls `start_in_cwd`).
- Add the `history` (and optionally `changes`) field to
  `TerminalWebSocketSession::start` and write per command batch.
- Port the `discovery.rs` advert of the history paths.

Caveat that survives the port: it still writes to `roots.static_root` on host
disk and only observes that host tree. Treat it as a `serve --root DIR`
convenience, never a mesh feature.
