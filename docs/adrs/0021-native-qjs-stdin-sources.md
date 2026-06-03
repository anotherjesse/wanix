# ADR 0021: Native qjs Stdin Sources

## Status

Accepted.

## Context

Rust Wanix qjs tasks already attach fd 0 when the composition layer supplies a
Wanix task stdin file. The native CLI exposed `--stdin TEXT`, which is useful for
tests, but it did not let host shell input flow into a qjs task.

That made the outside-Chrome demo less shell-composable than a normal native
process: `printf ... | wanix-rust qjs ...` could not reach QuickJS/WASI fd 0.

## Decision

Keep `--stdin TEXT` and add `--stdin-file PATH|-` to `wanix-rust qjs`.

`--stdin-file PATH` reads binary bytes from the host file before starting the qjs
task. `--stdin-file -` reads the native process stdin stream. Both forms install
the bytes as Wanix task fd 0, so guest JavaScript reads them through normal
QuickJS/WASI APIs such as `qjs:os.read(0, ...)`.

Only one stdin source is accepted per qjs command.

## Consequences

The CLI can now demonstrate host pipe input flowing through Wanix task fd 0 into
QuickJS outside Chrome.

The current implementation captures stdin bytes before task start. Streaming,
interactive terminals, and async lifecycle behavior remain future work.
