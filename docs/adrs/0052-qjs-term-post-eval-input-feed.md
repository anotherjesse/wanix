# ADR 0052: qjs-term Post-Eval Input Feed

## Status

Accepted

## Context

`wanix-rust qjs-term` can bind QuickJS task fd 0/1/2 through `#term`, and the
ready-IO demo proves preloaded terminal input can trigger `qjs:os` fd handlers.
That is still a transcript pattern: all input exists before JavaScript
evaluation starts.

Interactive shells need the next lifecycle shape. JavaScript should be able to
register a read handler, stay alive, receive terminal data later, and then run
ready-IO handler turns against the live Wanix task fds.

## Decision

Add `qjs-term --feed-after-eval TEXT`, accepted before the script path and
repeatable. When one or more post-eval feeds are supplied, `qjs-term` evaluates
the script without initial ready-IO turns, writes the feed chunks to
`#term/<id>/data`, then runs the configured `--ready-io-turns` against the still
live `QuickJsTaskRuntime`.

Expose a small `QuickJsTaskRuntime::run_ready_io_turns` method so composition
layers can pump task fd readiness without reaching through to the raw QuickJS
engine API.

## Consequences

The terminal CLI can now demonstrate input arriving after the script registers a
read handler. This is not a native TTY streaming loop yet, but it proves the
core lifecycle needed by one: create a qjs task runtime, evaluate setup code,
feed terminal bytes later, and explicitly drive ready-fd handlers while Wanix
task identity, namespace, and fds remain attached.
