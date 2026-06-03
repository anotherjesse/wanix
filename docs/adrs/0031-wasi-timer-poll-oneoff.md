# ADR 0031: WASI Timer Poll Oneoff

## Status

Accepted

## Context

The checked-in QuickJS fixture imports WASI Preview 1 `poll_oneoff`.
QuickJS-NG exposes synchronous timer behavior such as `qjs:os.sleep(...)`
through that import. The previous engine fallback only handled zero
subscriptions and returned `NOSYS` for non-empty polls, so guest timer calls
could not run outside Chrome even though they do not require Wanix filesystem,
task identity, or fd policy.

QuickJS-NG also has async timer-shaped APIs in some builds, such as
`sleepAsync`, `setTimeout`, and `setInterval`. At the time of this ADR, those
APIs needed an event loop that could hold pending host timer state, wake the
runtime later, and integrate with task cancellation and fd readiness. Draining
the QuickJS microtask queue alone was not that event loop.

Full polling is broader than this need. At the time of this ADR, read/write fd
readiness, async event loops, signals, cancellation, and integration with Wanix
task scheduling were process lifecycle semantics still outside the timer-only
slice.

## Decision

Support timer-only Preview 1 `poll_oneoff` in `wanix-qjs-engine`.

The engine rejects an empty subscription set as `EINVAL`, matching Preview 1's
empty-poll behavior. It accepts a single clock subscription for realtime or
monotonic clocks, sleeps the current host thread for relative timeouts, treats
absolute timeouts as due when they are at or before
`QuickJsHostConfig::clock_time_ns()`, and writes one clock event with the
guest-provided userdata.

This timer-only decision did not extend `QuickJsWasiHost` because the accepted
synchronous timer shape does not need Wanix namespace, fd, task, or process
policy. ADR 0035 later adds immediately-ready fd read/write events through live
WASI providers rather than hiding fd policy inside deterministic host config.

## Consequences

JavaScript running on the bundled QuickJS/WASI fixture can call
`qjs:os.sleep(...)` outside Chrome. Zero-delay sleeps are fast enough for tests;
non-zero sleeps block the current host thread.

This did not initially implement `sleepAsync`, `setTimeout`, `setInterval`,
signal delivery, task cancellation, or a Wanix scheduler. ADR 0040 later adds
bounded immediate event-loop turns for due async timers, and ADR 0042 adds an
explicit bounded wait budget for future timer demos. ADR 0044 later proves
self-clearing intervals inside that bounded pump. Scheduler integration remains
out of scope. Snapshot bytes remain QuickJS/Wasm memory only; pending async
timer state is still out of scope for persisted task policy.
