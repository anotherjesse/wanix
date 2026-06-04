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

Terminal-backed QuickJS tasks are one client of that service contract. The
terminal decision should cover device shape, readiness, resize delivery, and
session lifecycle; the QuickJS process boundary is covered by ADR 0002.

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

Expose terminal-backed task sessions through composition layers:

- `wanix-rust qjs-term` runs a caller-supplied script as a Wanix `qjs` task
  with fd 0/1/2 bound through `#term/<id>/program`.
- `wanix-rust qjs-shell` runs the bundled QuickJS shell source through the same
  terminal-backed task shape as the direct native shell demo.
- `serve --wanix-services` exposes a qjs-shell WebSocket route for
  browser/workbench pseudoterminals using the same terminal-backed task shape,
  with an optional Wanix cwd selected by the client.

Native and served shell loops may stream terminal output during eval, after
input batches, after ready-IO turns, and while reporting errors. They may pump
bounded guest event-loop work while the shell is otherwise idle so delayed
output can surface without another input frame or byte. These pumps are host
lifecycle policy for an already evaluated task runtime; they are not a general
Wanix scheduler.

For native shell input:

- cooked `qjs-shell` sessions remain line-oriented;
- `qjs-shell --raw` may put host stdin into a restore-on-drop raw-ish terminal
  mode when stdin is a TTY;
- raw mode feeds native bytes directly into `#term/<id>/data`; and
- the bundled QuickJS shell owns echo, simple editing, newline handling, Ctrl-D
  exit, and command dispatch when `WANIX_QJS_SHELL_RAW=1` is present.

Terminal resize travels through `#term/<id>/winch` as `columns rows\n`.
Deterministic fixtures, served shell sessions, native Unix shell sessions, and
workbench pseudoterminals should all use that same file. WASI cwd remapping
must not re-root `#term` service paths. Signal-driven resize wakeups may be
added later without changing the terminal device contract.

For editor-facing lifecycle, workbench pseudoterminals should close from Wanix
task/session state instead of from frontend-only assumptions. Served shell
routes can report task exit as lifecycle frames, and direct service-backed
workbench terminals may observe `#task/<id>/exit` over 9P after draining
terminal output.

## Consequences

Rust Wanix has one terminal contract that can be used by native CLI demos,
served browser/workbench sessions, and future VM/editor clients. JavaScript
running as a Wanix `qjs` task observes terminals through ordinary fd readiness
and service files such as `#term/<id>/winch`, while Wanix continues to own task
identity, fds, namespace, stdio, and exit state.

Exact native, served, and fixture behavior is pinned by tests and examples.
Future signal delivery, cancellation, process-group, durable attachment, or
VM/editor terminal policy should get a separate decision when it changes the
terminal/service contract.
