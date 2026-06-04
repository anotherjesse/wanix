# ADR 0050: Terminal Device and Shell Lifecycle

## Status

Accepted

## Context

Wanix terminals are a service contract, not browser xterm plumbing. The Go
runtime exposes terminals through `#term`: reading `#term/new` allocates a
resource id, and each resource exposes terminal-side data, program-side data,
and window-size notifications. Browser, native, served, editor, and VM clients
should all attach to that same device shape instead of inventing separate
process-specific terminal APIs.

The Rust port also needs terminal-backed QuickJS tasks outside Chrome. A `qjs`
task must keep Wanix ownership of task id, namespace, cwd/env/cmd, stdio/fds,
and exit state while QuickJS runs as the execution engine inside that task. The
terminal lifecycle therefore spans three boundaries:

- `wanix-term` owns the device filesystem and readiness behavior.
- `wanix-qjs`/`wanix-wasi` expose terminal-backed fds to QuickJS through live
  Wanix-owned WASI semantics.
- `wanix-cli` and `serve` compose task startup, native/browser input, output
  streaming, bounded event-loop pumps, and terminal resize delivery.

## Decision

Keep a Rust-native `wanix-term` crate that depends only on `wanix-fs` in
production code. It implements the terminal filesystem contract:

- `new` allocates incrementing terminal resource ids.
- `<id>/id` reports the resource id.
- `<id>/data` is the terminal/client side.
- `<id>/program` is the program/task side.
- `<id>/winch` broadcasts textual resize payloads as `columns rows\n` to open
  readers.

Writes to `data` are read from `program`. Writes to `program` are read from
`data`, with lone `\n` mapped to `\r\n` to match the terminal output behavior
expected by browser/editor terminal clients. Terminal read readiness is
queue-aware: terminal fds report readable only when bytes are queued for that
side or for that `winch` subscriber. Regular-file readiness stays default-ready
unless a file implementation overrides it.

Expose terminal-backed QuickJS demos through composition layers, not through a
separate QuickJS process model:

- `wanix-rust qjs-term` runs a caller-supplied script as a Wanix `qjs` task
  with fd 0/1/2 bound through `#term/<id>/program`.
- `qjs-term` supports deterministic post-eval feeds, line-segmented feeds, and
  resize feeds so tests and examples can prove input and `winch` events arriving
  after JavaScript has registered `qjs:os` handlers.
- `wanix-rust qjs-shell` runs the bundled QuickJS shell source as the direct
  native shell demo, preserving the same cwd/env/mount/runtime-limit options as
  other qjs task paths while rejecting script-path and preloaded-stdin fixture
  controls.
- `serve --wanix-services` exposes a qjs-shell WebSocket route that drives the
  same terminal-backed task shape for browser/workbench pseudoterminals.

Native and served shell loops may stream terminal output during eval, after
input/feed batches, after ready-IO turns, and while reporting errors. They may
also pump bounded QuickJS event-loop work while the shell is otherwise idle so
delayed guest output can surface without another input frame or byte. These
pumps are host lifecycle policy for an already evaluated task runtime; they are
not a general Wanix scheduler.

For native shell input:

- cooked `qjs-shell` sessions remain line-oriented;
- `qjs-shell --raw` may put host stdin into a restore-on-drop raw-ish terminal
  mode when stdin is a TTY;
- raw mode feeds native bytes directly into `#term/<id>/data`; and
- the bundled QuickJS shell owns echo, simple editing, newline handling, Ctrl-D
  exit, and command dispatch when `WANIX_QJS_SHELL_RAW=1` is present.

For terminal resize:

- deterministic fixtures write `columns rows\n` to `#term/<id>/winch`;
- served shell sessions forward browser resize messages through `winch`;
- native Unix shell sessions poll the host terminal size and deliver changed
  dimensions through `winch`;
- direct service-backed workbench pseudoterminals forward VS Code dimensions
  through `winch`; and
- signal-driven resize wakeups may be added later without changing the terminal
  device contract.

## Consequences

Rust Wanix has one terminal contract that can be used by native CLI demos,
served browser/workbench sessions, and future VM/editor clients. JavaScript
running as a Wanix `qjs` task observes terminals through ordinary fd readiness
and service files such as `#term/<id>/winch`, while Wanix continues to own task
identity, fds, namespace, stdio, and exit state.

Per-demo records for `qjs-term`, post-eval feeds, terminal fd readiness, native
output streaming, `qjs-shell`, raw byte pumping, deterministic resize feeds,
served idle pumping, and native idle/resize pumping are implementation history.
Their behavior is now pinned by tests and examples, while this ADR records the
durable terminal lifecycle contract.

This still leaves important future work: signal delivery, signal-driven resize
wakeups, cancellation, richer terminal/session lifecycle control, process
groups, durable terminal attachment, and VM/editor terminal policy.

## Replaces

This ADR consolidates ADRs 0050 through 0057, ADR 0091, ADR 0099, and the later
native fd-aware shell pumping and resize notes into the terminal device and
shell lifecycle contract.
