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
- Interim QuickJS fd helpers may demonstrate Wanix-owned task fd allocation
  before full WASI imports are available, but they must not create a separate
  QuickJS process or fd model.
- QuickJS task setup consumes `QuickJsWanixConfig`, which wraps
  `wanix_wasi::WasiConfig` for the current read-only projection adapter. Real
  WASI imports must continue to attach to Wanix task identity and fds rather
  than exposing the projection as an independent process model. The projection
  accepts only the root preopen so extra preopen/fd semantics cannot be
  accidentally flattened.
- `wanix-qjs` attaches open task stdio fds to `QuickJsWanixConfig` through
  private `wanix_fs::File` proxy handles. That keeps `wanix-wasi` generic while
  preserving Wanix task fd ownership for the future live WASI import path.
- `wanix-qjs` maps the task cwd to WASI fd 3 as the guest root preopen. The
  preopen still reports `/`, but bare WASI path calls resolve from the Wanix
  task cwd so `path_open("main.js")` matches `Wanix.readText("main.js")`.
- The qjs stdio proxies are live views by Wanix task fd number. Closing a task
  stdio fd affects the WASI attachment, while WASI `fd_close` still rejects
  stdio fds so lifecycle remains owned by Wanix task/fd APIs.
- The synchronous process demo comes before richer lifecycle work such as async
  event loops, signals, cancellation, and snapshot/restore.
- A useful demo should show JavaScript reading `#task/self/id`, using stdio,
  and leaving an observable task exit status.
