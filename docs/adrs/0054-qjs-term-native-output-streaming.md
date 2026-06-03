# ADR 0054: qjs-term Native Output Streaming

## Status

Accepted

## Context

`wanix-rust qjs-term` can keep a QuickJS task runtime alive after initial
evaluation, feed terminal input, and pump bounded ready-IO turns. Before this
decision, the native CLI still returned the terminal transcript only after the
task run completed. That preserved tests, but it meant a shell prompt written by
JavaScript was not emitted to the native process before the CLI tried to read
the next post-eval stdin line.

Interactive shell demos need native output to move independently from the final
task transcript. The Rust CLI also needs to keep its captured-output API for
unit tests and non-streaming commands.

## Decision

Add a public `wanix_cli::run_with_process_io` entrypoint that receives native
stdin/stdout/stderr streams and returns the process exit code. The existing
`run` and `run_with_process_stdin` APIs continue to return captured
`CliOutput`.

Use the process-IO entrypoint from the `wanix-rust` binary. For `qjs-term`, drain
`#term/<id>/data` into the supplied native stdout stream:

- after initial QuickJS evaluation,
- after each post-eval terminal feed and ready-IO pump,
- after task finish, and
- once more while reporting runtime errors.

Non-streaming commands still run through the captured-output path and write
their captured stdout/stderr before returning.

## Consequences

The native `wanix-rust qjs-term` path can now emit a shell prompt before it reads
the next post-eval native stdin line. This is a visible step from transcript
demos toward an interactive shell loop. Combined with ADR 0052's task-exit stop
condition for post-eval feeds, a shell command can also request process exit and
return without waiting for native stdin EOF.

This does not yet provide a full TTY event loop: native input is still driven by
the existing post-eval feed modes, there is no raw terminal mode, no concurrent
blocking wait on host input plus guest output, and no signal/window-size bridge.
Those remain the next shell-facing pieces.
