# ADR 0041: QuickJS Ready FD Handler Turn

## Status

Accepted

## Context

ADR 0035 made WASI `poll_oneoff` report immediately-ready fd read/write events
through live Wanix-backed providers. ADR 0040 added bounded immediate QuickJS
timer/job turns after `qjs` task evaluation. The bundled QuickJS fixture also
exports `qjs:os.setReadHandler(...)` and `qjs:os.setWriteHandler(...)`, but the
Wanix qjs runner did not yet call the QuickJS stdlib IO poll hook that invokes
those handlers.

The checked-in fixture exports `js_std_poll_io(ctx, timeout_ms)`. With a zero
timeout it asks the WASI poll import about registered fd handlers without
blocking.

## Decision

Expose `QuickJsRuntime::execute_ready_io_event_loop_once()`, backed by
`js_std_poll_io(ctx, 0)`.

After qjs task/script evaluation, run:

- bounded immediate QuickJS job/timer turns;
- one nonblocking ready-fd handler turn;
- bounded immediate job/timer turns again, so callbacks can resolve promises or
  schedule zero-delay async work.

The ready-IO turn is intentionally a single nonblocking turn. QuickJS's
`js_std_poll_io` reports the same success value whether no handler was ready or
one handler ran successfully, so Wanix cannot yet drain until idle without a
stronger fixture signal.

## Consequences

JavaScript running as a Wanix `qjs` task can now register a read handler on
ready stdin and observe that handler before task exit. The proof covers the
engine API, the qjs task driver, and a native CLI demo.

This does not implement long-lived fd scheduling, blocking readiness waits,
repeated handler draining, cancellation, signals, or a Wanix task scheduler.
Those remain future lifecycle decisions.
