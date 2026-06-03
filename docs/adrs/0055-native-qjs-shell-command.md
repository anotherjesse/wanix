# ADR 0055: Native qjs-shell Command

## Status

Accepted

## Context

The terminal shell demo previously required a long invocation:

```text
wanix-rust qjs-term --ready-io-turns 1 --feed-after-eval-lines - examples/qjs-term-shell-demo.js
```

That proved the runtime pieces, but it still looked like a scripted test harness
instead of a first-class Wanix shell demo. The next visible shell step is to make
the outside-Chrome QuickJS/WASI terminal path easy to run directly.

## Decision

Add `wanix-rust qjs-shell`. It runs the checked-in QuickJS shell source as a
bundled `qjs` Wanix task with:

- fd 0/1/2 bound through `#term/<id>/program`,
- terminal output streamed through the native process-IO path,
- native stdin delivered line-by-line after JavaScript evaluation,
- post-eval feeds stopped when the task requests process exit, and
- the same cwd/env/mount/runtime-limit options used by the qjs task runtime.

`qjs-shell` does not accept a script path, script arguments, `--stdin`, or
`--stdin-file`. Those remain `qjs`/`qjs-term` fixture controls. The shell command
is meant to be the direct native demo.

## Consequences

Wanix now has a simple command that demonstrates a QuickJS-backed task behaving
like an interactive shell outside Chrome:

```text
wanix-rust qjs-shell
```

The shell is still intentionally small. ADR 0056 adds native raw-mode setup and
host-side line editing, but guest input remains line-oriented. The next
shell-facing work is concurrent host input and guest-output waiting, signal
handling, resize propagation, and byte-oriented guest delivery where needed.
