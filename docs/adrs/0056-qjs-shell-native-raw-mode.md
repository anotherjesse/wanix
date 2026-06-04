# ADR 0056: qjs-shell Native Raw Mode

## Status

Superseded by [ADR 0091](0091-qjs-shell-raw-byte-pump.md) for guest input
delivery. Native raw-mode setup remains accepted.

## Context

`wanix-rust qjs-shell` made the QuickJS terminal shell easy to run outside
Chrome, but host stdin was still ordinary cooked input. That meant the host
terminal owned echo and line editing, and Wanix could not yet start taking
responsibility for terminal input behavior.

Full interactive terminal support still needs concurrent host input and guest
output waiting, signals, resize propagation, and richer line discipline. The
next safe step is to make raw host input explicit while preserving the current
line-oriented shell semantics.

## Decision

Add `wanix-rust qjs-shell --raw`.

When the native binary sees `qjs-shell --raw` and stdin is a TTY, it enters a
restore-on-drop terminal mode that disables canonical input and OS echo while
preserving signal generation. When stdin is not a TTY, the command skips the
termios change so tests and pipelines can still exercise raw-mode line
discipline deterministically.

In raw mode, the host side performs a small local line discipline:

- printable bytes are echoed immediately,
- backspace/delete edits the pending line and echoes the terminal erase
  sequence,
- enter echoes CRLF and sends the completed line to the Wanix terminal, and
- Ctrl-D ends input only when no line is pending.

The guest still receives complete lines through `#term/<id>/program`. Raw mode
is therefore a host terminal-input milestone, not yet a byte-at-a-time guest TTY
contract.

## Consequences

`qjs-shell --raw` lets the native terminal path own echo and simple editing
instead of relying on the host terminal's cooked mode. It moves the shell demo
toward real terminal control while keeping the Wanix task/runtime contract
unchanged.

The remaining shell work is to move beyond line completion: concurrent input and
guest-output waiting, byte-oriented guest delivery where appropriate, signal
handling, and resize propagation.
