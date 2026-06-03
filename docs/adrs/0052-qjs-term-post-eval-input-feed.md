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

Add post-eval terminal feeds accepted before the script path:

- `qjs-term --feed-after-eval TEXT` writes a literal byte chunk.
- `qjs-term --feed-after-eval-file PATH|-` writes one host file or native stdin
  byte chunk.
- `qjs-term --feed-after-eval-lines PATH|-` splits a host file or native stdin
  source into line-preserving chunks. When the source is native stdin (`-`),
  lines are read one at a time after JavaScript evaluation has completed.

When one or more post-eval feeds are supplied, `qjs-term` evaluates the script
without initial ready-IO turns, writes each feed batch to `#term/<id>/data`, then
runs the configured `--ready-io-turns` against the still live
`QuickJsTaskRuntime` after each batch. Contiguous literal/file feeds stay in one
batch for compatibility; line-segmented feeds create one batch per input line.
File-backed line feeds may be preloaded, but native stdin line feeds are
streamed after eval so a future interactive host loop does not need EOF before
delivering the first line.

Stop delivering post-eval terminal feeds once the live `QuickJsTaskRuntime`
reports a process exit. A guest read handler that calls `std.exit(...)` should
let the host-side line feed loop return without waiting for native stdin EOF.

Expose a small `QuickJsTaskRuntime::run_ready_io_turns` method so composition
layers can pump task fd readiness without reaching through to the raw QuickJS
engine API.

## Consequences

The terminal CLI can now demonstrate input arriving after the script registers a
read handler. The line-segmented feed mode gives tests and demos a deterministic
scripted-session harness where separate input lines become separate terminal
readiness events. Native stdin line feeds now run as a post-eval stream rather
than as a preloaded transcript. Together with ADR 0054's native output-drain
path, this proves the core lifecycle needed by an interactive loop: create a qjs
task runtime, evaluate setup code, emit output, feed terminal bytes later, and
explicitly drive ready-fd handlers while Wanix task identity, namespace, and fds
remain attached.
