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
  `wanix_wasi::WasiConfig` for live Wanix-backed WASI imports. Those imports
  attach to Wanix task identity and fds rather than exposing QuickJS as an
  independent process model.
- `wanix-qjs` attaches open task stdio fds to `QuickJsWanixConfig` through
  private `wanix_fs::File` proxy handles. That keeps `wanix-wasi` generic while
  preserving Wanix task fd ownership for the future live WASI import path.
- `wanix-qjs` maps the task cwd to WASI fd 3 as the guest root preopen. The
  preopen still reports `/`, but bare WASI path calls resolve from the Wanix
  task cwd so `path_open("main.js")` matches `std.loadFile("main.js")`.
- The qjs stdio proxies are live views by Wanix task fd number. Closing a task
  stdio fd affects the WASI attachment, while WASI `fd_close` still rejects
  stdio fds so lifecycle remains owned by Wanix task/fd APIs.
- `wanix-qjs` installs a qjs-compatible `scriptArgs` global from Wanix task
  argv. `scriptArgs[0]` is the program name and later entries are script
  arguments.
- A parent `qjs` task may allocate and start a child `qjs` task by using
  Wanix-backed WASI calls against `#task/new/qjs` and `#task/<id>/cmd`,
  `env`, `dir`, `ctl`, and `exit`. The child still gets Wanix task identity,
  namespace, argv/env/cwd, and exit state from `wanix-task`; QuickJS only
  executes the task driver. Child tasks do not implicitly inherit parent stdio
  fds. A parent wires child stdio explicitly before `start` by writing
  `bind <src> fd/<n>` to the child's `ctl` file. Rust resolves `<src>` in the
  target task namespace and installs it in the target task fd table; use an
  explicit source such as `#task/1/fd/1` when the child should write through a
  parent fd, because `#task/self` is target-task-relative.
- The synchronous process demo comes before richer lifecycle work such as async
  event loops, signals, cancellation, and snapshot/restore.
- A useful demo should show JavaScript reading `#task/self/id`, using stdio,
  and leaving an observable task exit status.
