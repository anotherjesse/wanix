# ADR 0053: Terminal FD Readiness

## Status

Accepted

## Context

Rust Wanix can now run QuickJS tasks through `#term` and can feed terminal input
after JavaScript registers `qjs:os` fd handlers. The remaining interactive-shell
path needs a host loop that can pump ready-IO turns while no native input is
available, without causing QuickJS read handlers to fire on empty terminal reads.

Before this decision, the QuickJS Preview 1 `poll_oneoff` import treated live
fd subscriptions as ready whenever the fd existed and had the requested rights.
That was acceptable for regular-file demos, but terminal devices need readiness
to reflect queued input.

## Decision

Add default-ready `read_ready` and `write_ready` hooks to Wanix open files, and
thread them through `WasiCtx`, the live QuickJS WASI host trait, and the engine
`poll_oneoff` import.

The default preserves existing regular-file and generic live-host behavior:
readable or writable fds still report ready immediately unless their host
overrides readiness. `wanix-term` is the first override: reads from
`#term/<id>/program` are ready only when the terminal data side has queued input,
and reads from `#term/<id>/data` are ready only when the program side has queued
output.

## Consequences

`qjs-term` can run ready-IO turns while a terminal read handler is armed but no
input is queued, and the handler will not fire until the host writes terminal
input. This moves the terminal path from transcript-only demos toward a real
interactive loop where native stdin/output streaming can drive a live Wanix task
runtime.
