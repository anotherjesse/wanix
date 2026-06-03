# ADR 0051: Terminal-Backed qjs CLI Demo

## Status

Accepted

## Context

Rust Wanix now has a `wanix-term` filesystem that implements the first
Rust-native `#term` device contract. The existing native `qjs` CLI command
proves QuickJS/WASI tasks outside Chrome, but it still captures stdio through
ordinary in-memory files. That proves process stdio, not terminal wiring.

The next visible step toward shells is to run the same QuickJS task model
through terminal fds. This should not replace the stable `qjs` command or
invent a separate QuickJS process model.

## Decision

Add a separate `wanix-rust qjs-term` demo command. It reuses the existing qjs
task setup, script staging, cwd/env/argv, host mounts, and runtime limits, then:

- allocates a `wanix-term` resource,
- binds the terminal device at `#term`,
- binds task fd 0, 1, and 2 to `#term/<id>/program`,
- writes any configured native stdin source to `#term/<id>/data` before start,
- runs the QuickJS task, and
- returns the `#term/<id>/data` transcript as native stdout.

The command is intentionally transcript-oriented for this slice. It proves the
terminal-backed task boundary without requiring a live native TTY event loop or
blocking terminal reads yet.

## Consequences

Rust Wanix now has an externally visible terminal-backed QuickJS task demo.
Guest JavaScript can use `qjs:std`, `qjs:os`, `scriptArgs`, `#task`, and
`#term` while fd-based stdio flows through the terminal device.

This is direct progress toward an interactive shell path, but it is not the
shell itself. Later cycles added post-eval input feeds and native output
streaming; the remaining shell-facing step is to replace scripted feed sessions
with a live native terminal input loop while the task runtime remains active.
