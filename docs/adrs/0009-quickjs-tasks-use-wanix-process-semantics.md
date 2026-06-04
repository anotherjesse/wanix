# ADR 0009: QuickJS Runs As A Wanix Task

## Status

Accepted

## Context

JavaScript should run outside Chrome, but that does not make QuickJS a separate
process model. Wanix already has task identity, `#task`, command files, env,
cwd, fd tables, fd binding, stdio, exit state, and driver registration. QuickJS
is the execution engine inside one task kind.

The important boundary is that Wanix owns process semantics even when the guest
language is JavaScript.

## Decision

Treat `qjs` as a real Wanix task driver. A QuickJS-backed task uses the same
task model as other Wanix tasks:

- `#task/new/qjs` allocates a task identity and service tree.
- `cmd` stores shell-quoted argv so paths, spaces, quotes, and empty arguments
  survive file-controlled task setup.
- `env`, `dir`, and cwd are Wanix task metadata, not QuickJS global state.
- `ctl bind` uses the same shell-word grammar as `cmd` for wiring fds and
  preserving Wanix paths with spaces.
- fd 0/1/2 and later fds are Wanix task fds that may be backed by memory files,
  namespace files, host mounts, terminal devices, or service-file handoffs.
- opening `#task/<id>/fd/<n>` captures a shared open-file handle so fd binds
  remain usable after the source task closes its fd.
- native qjs entrypoints can seed stdin from text, host files, or native stdin,
  but the resulting input is still installed as Wanix task fd 0.
- exit status is observable through Wanix task state.

The QuickJS adapter flows task argv, env, cwd, stdio, and fd state into the live
WASI provider and engine create/restore path. It does not expose a parallel
JavaScript process object.

## Consequences

QuickJS can provide the first serious Wanix runtime outside Chrome while the
task table remains the source of truth for process identity and lifecycle.
Tests and demos should prefer `qjs:std`, `qjs:os`, `scriptArgs`, stdio, and
`#task` service files over temporary helper globals.

Future lifecycle features such as async scheduling, cancellation, signals,
process groups, and snapshot-aware fd policies should extend Wanix task
semantics rather than QuickJS-specific process state.

## Replaces

This ADR consolidates ADR 0015, ADR 0019, ADR 0021, and ADR 0022 into the
QuickJS-as-Wanix-task process model.
