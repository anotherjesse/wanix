# ADR 0009: QuickJS Tasks Use Wanix Process Semantics

## Status

Accepted

## Context

QuickJS makes JavaScript execution possible outside Chrome, but Wanix already
has the process-shaped concepts that should define runtime identity and
capability boundaries: task id, `#task`, command, environment, working
directory, exit state, namespace, file descriptors, and driver registration.

Treating QuickJS as its own process model would split those semantics and make
WASI/fd behavior harder to reason about.

## Decision

JavaScript running in QuickJS is modeled as a Wanix task with task kind `qjs`.
QuickJS is the execution engine inside that task. Wanix owns task identity,
namespace, cwd/env/cmd, stdio/fds, and exit state.

`wanix-task` owns process/task semantics. `wanix-qjs` implements the adapter and
driver for `qjs` tasks. `wanix-wasi` provides Wanix-owned WASI syscall semantics
backed by task namespaces and fds. CLI and other composition layers wire the
driver into a task table.

## Consequences

- QuickJS must not own global process identity or fd semantics.
- Near-term `qjs` work should flow task `cmd`, `env`, and `dir` into
  QuickJS/WASI, then back WASI path/fd calls with the task namespace and fd
  table.
- The synchronous process demo comes before richer lifecycle work such as async
  event loops, signals, cancellation, and snapshot/restore.
- A useful demo should show JavaScript reading `#task/self/id`, using stdio,
  and leaving an observable task exit status.
