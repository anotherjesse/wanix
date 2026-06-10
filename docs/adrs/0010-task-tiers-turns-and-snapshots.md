# ADR 0010: Task Tiers, Turns, and Snapshot/Migration

## Status

**Proposed.** This is the task concurrency decision ADR 0002 deliberately did
not make ("bounded execution policy is not a general scheduler") — made now
because three roadmap items are blocked on the same missing model, and because
the operability goals in ADR 0000 (snapshot, migrate, inspect, fork) are
decided *here* or nowhere. Tier-2 foundations have shipped: bounded blocking
`#pipe` with broken-pipe honesty (`crates/wanix-pipe`), detached per-task host
threads via `ctl` `start &` plus the blocking `#task/<id>/wait` file
(`crates/wanix-task`), blocking `fd_read` and the minimal `poll_oneoff` subset
(`crates/wanix-wasi-host`), and concurrent shell pipelines plus the
interactive REPL (`crates/wanix-sh`), proven end to end in
`crates/wanix-wasm/src/driver.rs` tests. Tier-2 blocking now covers the qjs
layer too: `WasiCtx::fd_read` parks on stdio device fds via the shared
`wanix_wasi::wait` machinery, proven by a resident QuickJS stdin loop in
`crates/wanix-qjs`.

## Context

Three queued work items are the same problem wearing different clothes:

1. **Shell pipelines run sequentially** against the unbounded in-memory
   `#pipe` (stage A fully buffers, then B drains) — restoring Plan 9's
   concurrent stages needs tasks on their own threads and a bounded pipe.
2. **The interactive shell** needs a blocking `#term` read plus `poll_oneoff`
   in `wanix-wasi-host` on a dedicated thread.
3. **AppResource's host-pump design** (docs/appfs.md) exists entirely to keep
   a single-actor guest from becoming a blocked byte pump.

And one product requirement decides which way to jump: **a running task should
be snapshottable, migratable, and inspectable** — pull a misbehaving agent
from the home node to a laptop, inspect it, resume it; no snowflake servers.
The technical constraint: a Wasmtime instance cannot be portably snapshotted
while wasm frames are live on the stack. A task blocked deep inside a guest
`read()` is the *least* snapshottable artifact on the platform. A guest that
processes an event and returns — empty stack between turns — is snapshottable
by copying linear memory.

So the actor-turn model is not just a concurrency dodge: **the turn is the
migration-native task shape.**

## Decision

Three task tiers, by contract rather than by runtime:

### Tier 1 — turn-based resident tasks (agents, AppResources)

- Single-actor guest; the host delivers one event at a time (request, inbox
  write, job completion, timer); the guest handles it and **returns**.
- All blocking I/O and streaming is **host-owned**: the host holds the handle
  table (open files, streams, subscriptions) and pumps bytes; a parked reader
  parks a host queue, never the guest.
- The wasm stack is empty between turns. A turn boundary is therefore the
  **snapshot point, kill point, fork point, and migration point**.
- A tier-1 snapshot = linear memory + globals + the handle-table records
  (reified as `(path, offset, mode, filter)` descriptors) + Wanix task
  metadata (fd table, namespace binds, cwd/env, queued events).
- Migration restores the snapshot on another node and re-establishes mounts by
  **stable peer identity** — exactly the semantics ADR 0008 already pinned
  (identity is the authority, route hints are disposable, stale open handles
  are not resurrected: handle records are re-opened by path, and a handle that
  cannot re-open surfaces a typed error to the guest on its next use).

### Tier 2 — command tasks (POSIX-ish WASI guests)

- `wanix-sh`, `jaq`, arbitrary compiled `wasm32-wasi` programs, qjs scripts:
  blocking reads, `poll_oneoff`, sequential control flow.
- Each running tier-2 task gets its **own host OS thread**. Pipes become
  **bounded, blocking** channels; pipeline stages run concurrently (the Plan 9
  semantics the shell follow-up asks for).
- Killable (below) and exit-observable; **not migratable mid-run** — honest
  contract: commands are short-lived workers, not residents. (QuickJS VM
  snapshots per ADR 0002 remain VM images, not task migration.)

### Tier 3 — whole-OS guests (v86/QEMU)

The escape hatch when only Linux will do. Lifecycle and snapshotting are the
VM's own (v86 has save/restore); Wanix supplies the namespace via 9P
(ADR 0004 zone 3) and treats the VM as a foreign peer.

### Cross-cutting rules

- **No async in guests; no guest threads.** Guest-visible concurrency is
  turns (tier 1) or blocking syscalls (tier 2). Host threads are an executor
  detail beneath the contract, never part of it.
- **The host owns pumping.** Splicing a stream from a mounted file to a
  client, fan-out to subscribers, backpressure — host work, with bounded
  buffers and explicit slow-consumer policy.
- **Kill is a task operation:** `#task/<id>/ctl` accepts `kill` — implemented
  by Wasmtime epoch interruption (already shipped as ADR 0002 bounded-
  execution policy), recording a killed exit through `Task::set_exit` and
  running the same fd-release path as normal exit (`task-exit-closes-fds`,
  which gives pipeline EOF on death for free). Kill promises **fd release and
  observable exit only** — no transactional cleanup of half-written device
  state. ToolFS `ctl abort`, AppResource client-disconnect, and the
  interactive shell's Ctrl-C-on-foreground-child all bottom out here.
  ADR 0003 is unchanged: `#term/<id>/ctl close` stays resource cleanup;
  death belongs to `#task`.
- ADR 0002's stance is refined, not reversed: there is still no general
  signal system or process-group model; the scheduler is exactly "turns for
  residents, a thread for each running command."

## Consequences

One model unblocks all three queued items: pipeline concurrency and the
interactive shell land on tier-2 threads + bounded pipes; AppResource lands on
tier-1 turns + the host handle table. Agents (ADR 0000) get their substrate:
pause, snapshot, migrate, fork at turn boundaries, with mesh mounts surviving
migration by peer identity.

Costs accepted: per-task thread accounting (a cap and a shutdown signal —
shares the serve-concurrency cleanup already queued); a snapshot format to
design and version (CAS-backed; its relationship to capsules is an open
question below); and two task shapes to document and test instead of one.

## Open questions

- **Is a tier-1 snapshot a capsule?** Both are CAS-rooted frozen artifacts;
  capsules freeze a world's files, snapshots add a memory image and handle
  records. If they unify, one loader and one catalog `cas:` story covers
  both. (Under active design discussion — see
  [docs/design/agent-native-next-layer.md](../design/agent-native-next-layer.md).)
- **Snapshot portability across engine versions:** a linear-memory image is
  coupled to the guest module (same CAS hash) but should not be coupled to a
  Wasmtime version the way compiled artifacts are (ADR 0002 cache). Pin: a
  snapshot references the *source wasm* by hash, never a compiled artifact.
- **Thread cap and shutdown:** the tier-2 thread-per-task model needs the
  serve-side shutdown-signal work before a cap is testable; do them together.
