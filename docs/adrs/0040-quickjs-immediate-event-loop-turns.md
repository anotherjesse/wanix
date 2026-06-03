# ADR 0040: QuickJS Immediate Event Loop Turns

## Status

Accepted

## Context

ADR 0031 made synchronous `qjs:os.sleep(...)` work by backing timer
`poll_oneoff`, and ADR 0035 added immediately-ready fd poll events through live
WASI providers. The bundled QuickJS fixture also exports async timer-shaped
APIs such as `os.sleepAsync(...)` and `os.setTimeout(...)`, but ordinary Wanix
`qjs` task execution only drained QuickJS promise jobs after evaluating the
script.

QuickJS-NG exposes `js_std_loop_once(ctx)`, which runs pending jobs, fires at
most one expired timer, and reports whether the runtime is idle, has immediate
work, or is waiting for a future timer.

## Decision

Expose a Rust engine API for `js_std_loop_once`:

- `QuickJsRuntime::execute_event_loop_once()` returns an idle, pending, or
  future-wait status.
- `QuickJsRuntime::execute_immediate_event_loop_with_limit(max_turns)` drains
  bounded immediate work without sleeping for future timers.

Use that bounded immediate event-loop pump after `qjs` task/script evaluation.
This lets due async timer callbacks, such as `os.sleepAsync(0)`, run and write
through Wanix-owned stdio/fds before the task exits.

## Consequences

JavaScript running as a Wanix `qjs` task can now demonstrate a first async
lifecycle slice outside Chrome. The proof covers the engine API, the qjs task
driver, and a native CLI example. QuickJS clamps `setTimeout(..., 0)` to a
future timer, so ordinary timeout wakeups remain part of scheduler and clock
policy rather than this immediate pump.

Future timers are initially reported as a wait status rather than slept in the
immediate task pump. ADR 0042 later adds an explicit bounded wait budget for
future timer demos, and ADR 0044 proves self-clearing intervals inside that
bounded pump. Long-lived timers, open-ended intervals, fd handler scheduling,
signals, cancellation, and a Wanix task scheduler remain future lifecycle work.

The event-loop state remains QuickJS VM state. Snapshot bytes continue to be VM
images only; callers still need to reject or deliberately define policy for
open host resources and future scheduled work before snapshotting richer tasks.
