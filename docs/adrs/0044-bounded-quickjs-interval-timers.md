# ADR 0044: Bounded QuickJS Interval Timers

## Status

Accepted

## Context

ADR 0042 added a bounded future-timer pump for `qjs:os.setTimeout(...)`.
QuickJS also exposes `qjs:os.setInterval(...)`, which can keep scheduling work
after each callback. Supporting intervals as an unbounded task lifecycle would
need scheduler, cancellation, and task liveness policy that Wanix does not have
yet.

The bounded timer pump can still prove a useful process-runtime slice when the
JavaScript interval callback clears itself before the wait budget or turn budget
is exhausted.

## Decision

Use the existing bounded future-timer pump for self-clearing intervals. A Wanix
`qjs` task may run `setInterval` callbacks outside Chrome when the composition
layer grants an explicit `--event-loop-ms` wait budget and the callback clears
the interval before the pump reaches its bounds.

No new scheduler API is added for this slice. Open-ended intervals remain a
task lifecycle decision for future scheduler work.

## Consequences

JavaScript running as a Wanix `qjs` task can now demonstrate repeated timer
callbacks through `setInterval(...)` while still exiting deterministically once
the interval is cleared.

This does not keep tasks alive indefinitely, serialize pending interval state in
snapshots, or define cancellation/signal policy for long-running interval
tasks.
