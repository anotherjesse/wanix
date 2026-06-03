# ADR 0042: Bounded QuickJS Future Timer Pump

## Status

Accepted

## Context

ADR 0040 let Wanix `qjs` tasks drain QuickJS work that was already due after
script evaluation. That made `os.sleepAsync(0)` useful, but ordinary
`os.setTimeout(..., delay)` still stopped at a future-wait status and the task
then exited.

QuickJS timer state lives inside the VM image, while the observed WASI clock is
host state. The engine's default clock is deterministic and fixed, so sleeping
the host thread alone does not make future QuickJS timers due.

## Decision

Expose `QuickJsRuntime::execute_event_loop_with_wait_budget(max_turns,
max_wait)`.

For each future timer delay inside the caller's budget, the engine sleeps the
current host thread, advances the runtime's WASI clock by that same duration,
and continues pumping jobs and expired timers. If the next future timer exceeds
the remaining budget, the pump returns a wait status instead of blocking
indefinitely.

`wanix-qjs` keeps a zero wait budget by default. `QuickJsTaskDriver` can opt in
with `with_event_loop_wait_budget(...)`, and `wanix-rust qjs` exposes that as
`--event-loop-ms N` for native demos. ADR 0049 later applies the same explicit
budget to persistent `qjs-snapshot` and `qjs-resume` invocations.

## Consequences

JavaScript running as a Wanix `qjs` task can now demonstrate a real future
`setTimeout` callback outside Chrome when the composition layer grants an
explicit wait budget. ADR 0044 later applies the same bounded pump to
self-clearing `setInterval` demos. The proof covers the engine API, qjs task
driver, and native CLI demo.

This is still not a Wanix scheduler. It does not keep tasks alive forever, does
not wait for fd readiness beyond ADR 0043's fixed nonblocking turn budget, and
does not add signals, cancellation, open-ended interval lifecycle policy, or
snapshot serialization for pending timer state. Those remain task lifecycle
work.
