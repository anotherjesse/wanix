# ADR 0057: qjs-term Post-Eval Resize Feed

## Status

Accepted

## Context

Rust Wanix has a `#term/<id>/winch` broadcast file, but the native qjs terminal
demo had no way to prove a running QuickJS task could observe terminal resize
events. Full native terminal integration still needs a host loop that watches
the real TTY for size changes and forwards them while guest output is also
streaming.

The Go/browser-era service treats `winch` as a broadcast channel for open
readers, and the VS Code terminal adapter sketch used the textual payload
format `columns rows\n`.

## Decision

Add an explicit `wanix-rust qjs-term --resize-after-eval COLSxROWS` feed. The
CLI parses the human-facing `COLSxROWS` value and writes `columns rows\n` to
`#term/<id>/winch` after JavaScript has evaluated. It then pumps the configured
ready-IO turns so a QuickJS `qjs:os.setReadHandler` on the winch fd can consume
the event.

`#term/<id>/winch` remains a non-sticky broadcast: resize writes are delivered
only to readers that are already open. The Rust terminal device now reports
winch read readiness only when that subscriber has queued bytes, matching the
queue-aware terminal stdin/stdout readiness behavior.

## Consequences

The Rust port has a deterministic terminal resize proof outside Chrome:
JavaScript running as a Wanix qjs task can subscribe to `#term/1/winch` and
receive a resize event through live Wanix-backed WASI fd readiness.

This does not yet watch native `SIGWINCH`, maintain current terminal dimensions
as sticky state, define a structured binary resize frame, or deliver byte-mode
terminal input. Those remain part of the broader interactive shell/terminal
loop work.
