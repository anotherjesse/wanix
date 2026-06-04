# ADR 0099: Serve qjs-shell Idle Pump

## Status

Accepted.

## Context

ADR 0096 added a `/.well-known/qjs-shell` WebSocket route for the Rust-served
workbench terminal. The route forwarded browser input and resize messages into
the QuickJS-backed Wanix task, but it only advanced the task after receiving a
browser WebSocket frame. Guest work that became ready while the terminal was
otherwise idle could remain latent until the next keystroke.

## Decision

Give `QjsShellSession` an explicit host pump that runs bounded QuickJS event-loop
work for an already evaluated Wanix task. The served WebSocket route installs a
short socket read timeout and treats timeout or would-block reads as idle ticks:
it pumps the session, sends any terminal output, and emits the same exit frame if
the guest exited.

The pump is composition-layer lifecycle policy. It does not move task identity,
stdio, namespace, or exit ownership into QuickJS; those remain Wanix task state.

## Consequences

The workbench pseudoterminal can now receive delayed guest output without a
follow-up browser input frame. This is still a bounded shell heartbeat, not a
general scheduler, but it moves the served qjs shell closer to a live
interactive process.
