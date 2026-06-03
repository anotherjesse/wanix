# ADR 0043: Bounded QuickJS Ready IO Turns

## Status

Accepted

## Context

ADR 0041 added one nonblocking QuickJS stdlib fd readiness turn after Wanix
`qjs` task evaluation. That proved `qjs:os.setReadHandler(...)` could observe a
ready Wanix task fd before process exit, but callbacks that intentionally keep a
handler installed could not run again without a scheduler.

The checked-in QuickJS fixture's `js_std_poll_io(ctx, timeout_ms)` returns the
same success value when no handler is ready and when one handler ran
successfully. Wanix therefore still cannot drain ready IO until idle.

## Decision

Keep the default qjs task policy at one nonblocking ready-IO turn.

Add an explicit fixed ready-IO turn budget:

- `QuickJsTaskDriver::with_ready_io_turns(turns)` sets the driver policy.
- `QuickJsRunner::run_task_with_event_loop_limits(...)` accepts the same count.
- `wanix-rust qjs --ready-io-turns N` exposes it for native demos.
- ADR 0049 later applies the same explicit turn budget to persistent
  `qjs-snapshot` and `qjs-resume` invocations.

After script evaluation, Wanix drains timer/job work, runs up to the configured
number of nonblocking ready-IO turns, and drains timer/job work after each turn
so callbacks can schedule immediate follow-up JavaScript.

## Consequences

JavaScript running as a Wanix `qjs` task can now demonstrate repeated
`setReadHandler` callbacks outside Chrome when the composition layer grants an
explicit turn budget. This advances fd-driven process lifecycle behavior without
claiming an idle signal the fixture does not provide.

This is still not streaming host stdin, blocking fd readiness, cancellation,
signals, or a Wanix task scheduler. Those require real readiness sources and
task lifecycle policy beyond this fixed nonblocking turn budget.
