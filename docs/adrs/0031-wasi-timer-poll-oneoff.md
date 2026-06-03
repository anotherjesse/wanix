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

Full polling is broader than this need. Read/write fd readiness, async event
loops, signals, cancellation, and integration with Wanix task scheduling are
process lifecycle semantics, not QuickJS engine plumbing.

## Decision

Support timer-only Preview 1 `poll_oneoff` in `wanix-qjs-engine`.

The engine rejects an empty subscription set as `EINVAL`, matching Preview 1's
empty-poll behavior. It accepts a single clock subscription for realtime or
monotonic clocks, sleeps the current host thread for relative timeouts, treats
absolute timeouts as due when they are at or before
`QuickJsHostConfig::clock_time_ns()`, and writes one clock event with the
guest-provided userdata.

The engine continues to reject fd read/write subscriptions and multi-event
polls as unsupported. `QuickJsWasiHost` is not extended for this cycle because
the accepted timer shape does not need Wanix namespace, fd, task, or process
policy. Future fd readiness or async timer integration should be added through
Wanix task lifecycle semantics, not hidden inside deterministic host config.

## Consequences

JavaScript running on the bundled QuickJS/WASI fixture can call
`qjs:os.sleep(...)` outside Chrome. Zero-delay sleeps are fast enough for tests;
non-zero sleeps block the current host thread.

This does not implement `sleepAsync`, `setTimeout`, signal delivery, fd
readiness, task cancellation, or a Wanix scheduler. Snapshot bytes remain
QuickJS/Wasm memory only; pending async timer state is still out of scope.
