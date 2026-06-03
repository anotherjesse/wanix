# ADR 0035: WASI Poll FD Readiness

## Status

Accepted

## Context

ADR 0031 made `poll_oneoff` useful for synchronous QuickJS sleep calls, but
kept fd read/write readiness unsupported. That preserved a clean timer-only
engine fallback, but it left a core event-loop substrate missing for future
interactive tasks and richer QuickJS/WASI lifecycle work.

Wanix already owns fd identity, rights, and path semantics through live
`QuickJsWasiHost` providers backed by `wanix-wasi`. Read/write readiness is
therefore runtime host state, not deterministic `QuickJsHostConfig` data.

## Decision

Extend `wanix-qjs-engine` `poll_oneoff` to report immediately-ready fd
read/write events when a live `QuickJsWasiHost` provider is attached.

The engine decodes Preview 1 fd-read and fd-write subscriptions, asks the live
provider for `fd_fdstat_get`, and reports:

- a successful fd event when the fd exists and has the requested `FD_READ` or
  `FD_WRITE` right;
- an event carrying `NOTCAPABLE` when the fd exists without that right;
- an event carrying the provider errno, such as `BADF`, when the provider
  rejects the fd.

Without a live provider, fd subscriptions remain unsupported so the
engine-owned virtual-file fallback does not grow process policy. Multiple
immediately-ready fd subscriptions can return multiple events in one poll, and
due clock subscriptions may be returned alongside them. Clock-only behavior
remains the ADR 0031 timer fallback.

## Consequences

This moves Wanix toward a real event-loop substrate without adding a scheduler
or blocking readiness model yet. Regular files, service files, and task stdio
can be treated as immediately ready according to the live provider's fd rights.
ADR 0041 later drives one nonblocking QuickJS `setReadHandler`/`setWriteHandler`
turn through this readiness substrate after `qjs` task evaluation, and ADR 0043
adds an explicit fixed ready-IO turn budget.

Fd blocking, signal delivery, cancellation, and task scheduler wakeups remain
future Wanix task lifecycle work.
