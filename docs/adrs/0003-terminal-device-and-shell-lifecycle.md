# ADR 0003: Terminal Device and Shell Lifecycle

## Status

Accepted

## Context

Wanix terminals are a service contract, not browser xterm plumbing. The Go
runtime exposes terminals through `#term`: reading `#term/new` allocates a
resource id, and each resource exposes terminal-side data, program-side data,
and window-size notifications. Browser, native, served, editor, and VM clients
should attach to that same device shape instead of inventing separate
process-specific terminal APIs.

This ADR covers the terminal device shape, readiness, resize delivery, and
session lifecycle boundary. QuickJS process semantics are covered by ADR 0002;
serve, workbench, v86, and QEMU handoffs are covered by ADR 0005.

## Decision

Keep a Rust-native `wanix-term` crate that depends only on `wanix-fs` in
production code. It implements the terminal filesystem contract:

- `new` allocates incrementing terminal resource ids.
- `<id>/id` reports the resource id.
- `<id>/ctl` accepts lifecycle commands. The initial command is `close`, which
  removes the resource from `#term` and invalidates existing handles.
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

Terminal-backed tasks bind fd 0/1/2 to the program side of a terminal. Human,
browser, editor, and VM clients attach to the data side. Resize travels through
`#term/<id>/winch` as textual `columns rows\n` payloads, and WASI cwd remapping
must not re-root `#term` service paths. Client disposal should write `close` to
`#term/<id>/ctl` when it owns the terminal resource; that is a resource cleanup
signal, not a task cancellation or process-group signal.

Composition layers may provide cooked or raw native shells, browser/workbench
pseudoterminals, deterministic fixtures, or future VM/editor terminals. Those
surfaces should use `#task` and `#term` service files and close from observed
Wanix task/session state rather than from frontend-only assumptions. In raw
interactive modes, the host should feed bytes through the terminal device and
let the guest-side shell own echo, simple editing, newline handling, Ctrl-D, and
command dispatch.

The bundled QuickJS shell is a guest program inside a Wanix task, not a separate
process model. It may provide built-in interactive commands such as `cd`, `ls`,
`cat`, and `write`, but those commands should use `qjs:std`, `qjs:os`, and
service files. For shell sessions, `#task/self/dir` is the logical shell cwd,
while the WASI root preopen stays at the namespace root so `cd` can navigate the
served tree. Non-shell qjs script tasks continue to use task cwd as their WASI
root preopen.

Bounded output draining or event-loop pumping around terminal sessions is host
lifecycle policy for an already evaluated task runtime. It is not a general
Wanix scheduler, signal system, cancellation model, or process-group contract.

## Consequences

Rust Wanix has one terminal contract that can be used by native CLI demos,
served browser/workbench sessions, and future VM/editor clients. JavaScript
running as a Wanix `qjs` task observes terminals through ordinary fd readiness
and service files such as `#term/<id>/winch`; clients can release terminal
resources through `#term/<id>/ctl`, while Wanix continues to own task identity,
fds, namespace, stdio, and exit state.

Exact command, route, and fixture coverage belongs in tests, examples, and
current-state docs. Future signal delivery, cancellation, process-group,
durable attachment, or VM/editor terminal policy should get a separate decision
when it changes the terminal/service contract.
