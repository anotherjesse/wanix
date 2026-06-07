---
title: Bounded Execution Policy (not a scheduler)
slug: concepts/bounded-execution-policy
pageType: concept
oneLiner: The qjs driver carries four knobs — event-loop wait budget, ready-IO turns, interrupt poll budget, and memory limit — that bound a guest run, but they are not a scheduler, signal system, or cancellation model.
audience: [developer]
tags: [task-runtime, qjs, shipped, caveat, local-trust-only]
sourceRefs:
  - crates/wanix-qjs/src/driver.rs:11-102
  - crates/wanix-qjs/src/runner.rs:196-262
  - crates/wanix-qjs/src/runner/control.rs:15-58
  - crates/wanix-qjs/src/runtime_control.rs:42-66
  - crates/wanix-qjs-engine/docs/architecture.md:98-111
  - docs/scaling-eli5.md:89-95
seeAlso:
  - concepts/qjs-task
  - concepts/wasmtime-as-substrate
  - concepts/safe-for-untrusted-not-claimable
  - concepts/rooms-not-houses
prerequisites:
  - concepts/qjs-task
usedInFlows: []
honestLimits:
  - The knobs bound one guest run; they are not a general scheduler, signal system, or cancellation model.
  - Hard CPU/memory preemption (Wasmtime epoch/fuel, linear-memory caps) is not wired yet, so exec stays local-trust only.
  - ready_io_turns is a fixed turn count because QuickJS cannot tell an idle fd poll from one that ran a handler.
canonicalCaveatFor: []
---

# Bounded Execution Policy (not a scheduler)

The qjs driver carries four knobs — event-loop wait budget, ready-IO turns, interrupt poll budget, and memory limit — that bound a guest run, but they are not a scheduler, signal system, or cancellation model.

A QuickJS task runs to completion inside the host thread that started it. Left unbounded, a `while(true){}` would wedge that thread forever, and a runaway allocator would grow the heap until the process died. So the driver hands the engine a small, explicit policy before each run. These four numbers are exactly that policy — and nothing more. They make demos and tests deterministic; they do not turn Wanix into an operating-system scheduler.

## The four knobs

All four live on `QuickJsTaskDriver` and default to "do the least surprising thing" (`crates/wanix-qjs/src/driver.rs:11-31`):

- **`event_loop_wait_budget: Duration`** — how long the driver may block waiting for *future* QuickJS timers (`qjs:os` `setTimeout` and friends) after the top-level script returns. Default `Duration::ZERO`: drain only work that is already due, then exit. A nonzero budget is the difference between "fire-and-forget" and "let the timers actually land."
- **`ready_io_turns: usize`** — how many nonblocking ready-fd handler turns run after eval. Default `1`. It is a fixed turn count, not a "run until idle" loop, because QuickJS's stdlib fd poll hook cannot tell an idle poll from one that ran a callback (`crates/wanix-qjs/src/driver.rs:43-52`).
- **`interrupt_poll_budget: Option<usize>`** — a ceiling on QuickJS interrupt polls during eval. Default `None` (no ceiling). When the budget is exhausted, the interrupt handler returns `true` and QuickJS aborts the running script (`crates/wanix-qjs/src/runner/control.rs:36-57`). This is the only thing standing between you and a CPU-bound `while(true)`.
- **`memory_limit_bytes: Option<u32>`** — the QuickJS heap allocation ceiling. Default `None`. When set, the runner calls `runtime.set_memory_limit(bytes)` before the script runs (`crates/wanix-qjs/src/runner/control.rs:21-23`).

Show, then name: a test that wants a guest to time out instead of hang sets `with_interrupt_poll_budget(n)`; a demo that wants `setTimeout` callbacks to actually fire sets `with_event_loop_wait_budget(d)`. The "policy" is just those builder calls.

## What they bound, and why they exist

Each knob exists for a concrete, observable reason, not for completeness:

The **wait budget** and **ready-IO turns** together decide how much of the event loop runs *after* the synchronous script body finishes. Zero budget plus one turn is the original nonblocking lifecycle: eval, drain what is due, set the exit status, return. That makes a one-shot script deterministic — the same input always produces the same captured stdout and the same exit code. Bump the budget when a test or demo genuinely needs deferred timers to resolve.

The **interrupt budget** bounds *CPU-bound* JavaScript. The handler is a closure QuickJS calls periodically; when polls exceed the budget it asks QuickJS to stop (`crates/wanix-qjs/src/runtime_control.rs` wires the same handler so a requested exit also short-circuits the drain loop). It is a guardrail for tests that must not hang, not a quota you charge a tenant.

The **memory limit** bounds allocation-heavy guests the same way: a host policy applied per run, not a per-task budget you meter over time.

The driver threads all four into one call — `run_task_with_runtime_limits` — which builds the WASI config, reads the script from the task's namespace, runs it under these limits, and records the guest exit through `Task::set_exit` (`crates/wanix-qjs/src/runner.rs:196-262`). On error it still writes captured output and sets exit `"1"` (`crates/wanix-qjs/src/driver.rs:87-101`).

## Why this is explicitly NOT a scheduler

It is tempting to read four knobs as the start of a process model. It is not. The engine architecture notes are blunt about the boundary: these are "bounded execution controls around QuickJS" — job draining with limits, ready-IO turns, wait-budgeted loops, interrupt handlers, memory and stack limits (`crates/wanix-qjs-engine/docs/architecture.md:98-111`). What is deliberately absent:

- **No scheduler.** Nothing time-slices between tasks or arbitrates CPU across runs. A task owns its thread until it returns.
- **No signal system.** The interrupt budget aborts *this* run when *its own* count is exhausted; it is not a `SIGINT` you can deliver to a task from outside. Cancellation across tasks is unbuilt (see the queued follow-ups).
- **No cancellation model.** There is no handle a caller holds to stop a running guest mid-flight beyond the budget the driver pre-armed.
- **No checkpoint format.** These knobs are runtime policy, not state. They are not part of a snapshot; QuickJS snapshots are VM images of linear memory, a separate concern.

The interrupt budget *is* a per-run ceiling — but it is set once, before the run, and is per QuickJS context. Two concurrent tasks do not share a budget, and the host does not preempt either of them.

## Status / honest limits

The hardest limit is the one not yet built. These knobs are *soft, in-engine* policy: the interrupt handler only fires when QuickJS chooses to poll it, and the memory limit is QuickJS's own heap accounting. There is **no hard CPU/memory preemption** — Wasmtime epoch interruption, fuel metering, and linear-memory caps — wired into the qjs path yet (`docs/scaling-eli5.md:89-95`). Per-room memory isolation is real today; the hard limits that would make a room safe for *arbitrary untrusted code* are not.

That is exactly why the exec devices — `#task`, `#agent`, `#cpu` — stay **local-trust only** and are not exposed to untrusted or public peers. The honest framing is "cheap, scalable isolation," not "safe to run code you do not trust." Until epoch/fuel preemption lands, treat the four knobs as deterministic-demo and test guardrails, not a security boundary.

## See also

- [The qjs task](/concepts/qjs-task) — what a QuickJS task is and how the driver starts it.
- [Wasmtime as substrate](/concepts/wasmtime-as-substrate) — where hard epoch/fuel preemption would eventually live.
- [Safe for untrusted is not claimable](/concepts/safe-for-untrusted-not-claimable) — why exec stays local-trust until the hard limits land.
- [Rooms, not houses](/concepts/rooms-not-houses) — the isolation model these knobs sit inside.
