# ADR 0091: qjs-shell Raw Byte Pump

## Status

Accepted

## Context

`wanix-rust qjs-shell --raw` already disabled host canonical input and echo when
stdin was a TTY, but the Rust CLI still performed a host-side line discipline.
It echoed printable bytes, handled backspace, waited for a completed line, then
fed that whole line into `#term/<id>/data`.

That made the demo look interactive, but QuickJS did not actually receive
individual keystrokes. The guest shell could not own echo/editing behavior, and
control bytes such as Ctrl-D were consumed by the host loop instead of reaching
the Wanix task.

## Decision

For `qjs-shell --raw`, feed native stdin bytes directly into the terminal device:

- read one byte at a time from native stdin after the shell has registered its
  read handler;
- write each byte to `#term/<id>/data`;
- pump QuickJS ready-IO after each byte; and
- drain terminal output after each pump.

Move the small shell echo/editing policy into the bundled QuickJS shell when
`WANIX_QJS_SHELL_RAW=1` is present. Cooked `qjs-shell` sessions and explicit
`qjs-term --feed-after-eval-lines` fixtures remain line-oriented so their
existing deterministic transcript behavior is preserved.

## Consequences

The raw shell demo now proves that a QuickJS-backed Wanix task receives terminal
input bytes outside Chrome. The guest shell owns visible echo, backspace/delete,
newline handling, Ctrl-D exit, and command dispatch.

This is still not a full terminal scheduler. Future work needs concurrent
waiting for native input and guest output, signal delivery, live resize
propagation, and richer terminal modes.
