---
title: "#task — The Task Device"
slug: devices/task
pageType: device
oneLiner: "Allocate, configure, start, and observe processes purely through files: read #task/new/<kind>, write cmd/env/dir, bind fds and start through ctl, then read exit."
audience: [developer]
tags: [task-device, process-lifecycle, fds, shipped, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-task/src/task_fs.rs
  - crates/wanix-task/src/table.rs
  - crates/wanix-task/src/task_files.rs
  - crates/wanix-task/src/task_files/control.rs
  - crates/wanix-task/src/task_files/field.rs
  - crates/wanix-task/src/task_files/new_task.rs
  - crates/wanix-task/src/task/fd_ops.rs
  - examples/qjs-task-spawn.js
  - examples/lib/wanix/task.js
  - rust-walkthrough.md
seeAlso:
  - concepts/tasks-own-process-identity
  - concepts/task-drivers
  - concepts/the-fd-table
  - concepts/blocking-stream-eof-contract
prerequisites:
  - concepts/tasks-own-process-identity
  - concepts/task-drivers
usedInFlows:
  - {flow: js-outside-chrome, step: 1}
honestLimits:
  - "Exec is local-trust: qjs/wasm drivers run in-process on the host; #task does not run code across the mesh trust boundary (cross-node compute is the separate #cpu exec plane)."
  - "No signals, no process groups, no kill: the only ctl verbs are bind and start, and a running guest cannot be cancelled through #task."
  - "Child launches are synchronous and foreground: a started child runs to completion on the controlling thread before start returns; there is no background task supervisor."
  - "exit is free-form text written by the driver (numeric code for qjs/wasm); the device imposes no schema."
canonicalCaveatFor: [exec-local-trust-only]
---

# `#task` — The Task Device

`#task` is how Wanix turns "start a process" into "write some files." There is
no `fork`, no `exec` syscall, no PID returned from a kernel call. Instead you
read `#task/new/<kind>` to allocate a task, write `cmd`/`env`/`dir` to configure
it, write `bind` and `start` lines to its `ctl` file to wire its fds and run it,
then read its `exit` file to learn how it finished. This is Plan 9's
process-as-a-directory idea, made literal: the entire task lifecycle is a small
tree of regular files, so anything that can open a file — a qjs guest, a shell,
the cockpit over 9P, or a peer across the mesh — can drive it the same way.

## The directory shape

A `#task` namespace looks like this. Numbered directories are live tasks; `new`
is the allocator; `self` is the calling task's own view.

```text
#task/
  new/          # allocator — one entry per driver kind
    auto
    noop
    qjs
    wasm
  1/            # an allocated task, by id
    id          # read-only: "1"
    kind        # read-only: "qjs"
    cmd         # read/write: argv as a command line
    env         # read/write: newline-separated KEY=value
    dir         # read/write: working directory (default ".")
    exit        # read/write: exit status text
    ctl         # write-only verbs: "bind ...", "start"
    fd/         # open fd numbers for this task
      0
      1
      2
  self          # alias for the calling task (only when there is one)
```

The set of `new/<kind>` entries comes straight from the registered driver
kinds, with `auto` always offered first
(`crates/wanix-task/src/table.rs`, `driver_kinds`). The per-task field set
(`id`, `kind`, `cmd`, `env`, `dir`, `exit`, `ctl`, `fd`) is fixed in
`task_files.rs` (`task_entries`).

## `#task/new/<kind>` — allocate a task

Allocation happens by **reading** `#task/new/<kind>`. The read returns the new
task's id followed by a newline; the allocation is performed lazily on that
first read (`crates/wanix-task/src/task_files/new_task.rs`). Writing to a `new`
file is `PermissionDenied` — there is nothing to write.

```text
$ read #task/new/qjs
2
```

`<kind>` must be a registered driver kind or `auto`:

- `noop` — allocates and "runs" trivially; the default driver used by tests and
  early allocation flows (`crates/wanix-task/src/driver.rs`).
- `qjs` — QuickJS/WASI task (registered by `wanix-qjs`).
- `wasm` — compiled `wasm32-wasi` task (registered by `wanix-wasm`).
- `auto` — defer the choice: at `start`, the table walks the registered drivers
  and picks the first whose `check(&task)` accepts the task (e.g. a `.wasm`
  `cmd`), then sets `kind` accordingly (`table.rs`, `start`).

If you read `#task/new/qjs` from a task that has a current identity, the new
task is allocated as a **child** of the caller and inherits a clone of the
caller's namespace; otherwise it is a **root** task with the namespace of the
view (`new_task.rs` → `allocate_child` / `allocate_root` in `table.rs`).

## `cmd`, `env`, `dir` — file-oriented task control

These three fields are plain read/write text files (mode `0755`). You configure
a task by writing them before you start it; you can read them back to inspect
state. Reads always end with a trailing newline (`task_files/field.rs`).

```text
$ write #task/2/cmd "main.js alpha 'two words' beta"
$ write #task/2/env "MODE=spawned\nVERBOSE=1"
$ write #task/2/dir "."
```

- `cmd` is parsed into argv with shell-style quoting on write; an unterminated
  quote is rejected. The raw text and the parsed argv are both retained
  (`Task::set_cmd`).
- `env` is split on newlines into `KEY=value` lines; an empty write clears it
  (`Task::set_env_lines`).
- `dir` must be a normalized path and defaults to `.` for allocated tasks
  (`task/state.rs`).
- `id` and `kind` are read-only (`0555`); writing them is `PermissionDenied`.

These fields are intentionally separate from a task's launch `spec`: writing
`cmd`/`env`/`dir` is the file-oriented control surface and does not mutate an
explicitly-set spec (`Task::set_spec` doc comment in `task.rs`).

## `#task/<id>/ctl` — bind fds, then start

`ctl` is the verb channel. Reads return zero bytes (EOF); writes are commands
that take effect immediately and the buffer is cleared on each accepted command
(`crates/wanix-task/src/task_files/control.rs`). Two verbs exist today:

**`bind <source> fd/<n>`** opens a namespace path and installs it at fd `<n>`
in the task's fd table. The destination must name an fd of the task you are
controlling — `fd/<n>`, `#task/self/fd/<n>`, or `#task/<this-id>/fd/<n>`; any
other task id is rejected (`control_fd_destination`). fd `0` is opened
read-only; all other fds read/write (`task/fd_ops.rs`, `fd_bind_open_options`).

**`start`** runs the task through its registered driver. For `auto`, the driver
is selected at this moment.

```text
$ write #task/2/ctl "bind 'child stdin.txt' fd/0"
$ write #task/2/ctl "bind #task/1/fd/1 fd/1"
$ write #task/2/ctl "bind #task/1/fd/2 fd/2"
$ write #task/2/ctl "start"
```

Partial writes are tolerated: an empty or incomplete command line (e.g. a
prefix of `start`, or a `bind` with too few words, or an unterminated quote)
parks as *pending* until the rest arrives, so streaming a command in chunks is
safe.

## `#task/<id>/exit` — observable exit status

`exit` is a read/write text file. A driver records the guest's exit status by
writing it (`Task::set_exit`); a controller reads it to wait for and observe the
result. The value is free-form text — for qjs/wasm it is the numeric exit code.

```text
$ read #task/2/exit
5
```

The guest SDK's `wait()` is exactly this read:
`parseInt(readText("#task/<id>/exit"))` (`examples/lib/wanix/task.js`). Reading
`exit` follows the device's stream contract until the status is available; see
the blocking-stream EOF reference below.

## `#task/self` — every task sees its own view

When a task is allocated, the table binds a `#task` filesystem **scoped to that
task** at `#task` (replace) inside the new task's namespace
(`table.rs`, `allocate_task`). That scoped view carries a `current` id, so the
task can refer to itself as `#task/self` without knowing its own number. The
`self` entry only appears when the view has a current task
(`task_fs.rs`, `root_entries`); a detached/admin view of the table sees the
numbered tasks but no `self`.

```text
$ read #task/self/id
1
$ read #task/self/cmd
main.js
```

This is what makes the spawn idiom composable: a parent wires a child's stdout
to its own by binding `#task/<parent>/fd/1`, and a child can always find itself
through `#task/self`.

## `#task/self/fd/<n>` — selected dynamic fds

The `fd/` directory lists the task's currently open fd numbers, and each entry
proxies reads/writes through to the underlying open file
(`task_fs.rs`, `read_dir`; `task_files.rs`, `FdProxyFile`). Not every WASI fd
appears here — only fds Wanix tracks in the task fd table. As the walkthrough
notes, dynamic regular-file WASI fds are mirrored into the task fd table at the
same number, which is precisely why `#task/self/fd/<n>` can expose selected
guest fds while Wanix still owns global fd semantics
(`rust-walkthrough.md`). Binding stdin/stdout/stderr through `ctl` is what makes
`fd/0`, `fd/1`, `fd/2` show up.

## Putting it together

The `qjs-task-spawn` example (`examples/qjs-task-spawn.js`) drives the whole
lifecycle from a guest, using the SDK's `spawn` helper which encodes the exact
file writes described above:

```js
import { spawn } from "lib/wanix/task.js";
import { self } from "lib/wanix/process.js";

const parent = self.id();
const child = spawn("qjs", {
  cmd: "qjs-task-spawn-child.js alpha 'two words' '' beta",
  env: { MODE: "spawned" },
  dir: ".",
  binds: [
    ["child stdin.txt", 0],
    ["#task/" + parent + "/fd/1", 1],
    ["#task/" + parent + "/fd/2", 2],
  ],
});
child.start();
print("child exit: " + child.wait());   // -> child exit: 5
```

`spawn` does *not* auto-start — it allocates, configures, and binds, then hands
back a `Task` so the caller can print ids before running it, mirroring the
file-level flow (`examples/lib/wanix/task.js`).

## Status and honest limits

`#task` is deliberately small. Know what it does not do:

- **Exec is local-trust.** The qjs/wasm drivers run in-process on the host. Real
  code execution is not exposed across the mesh trust boundary by `#task`
  itself; cross-node compute is the separate `#cpu` exec plane, which runs a
  task against a caller's reverse-exported namespace.
- **No signals, no process groups.** There is no `kill`, no `SIGTERM`, no job
  control through `ctl`. The only verbs are `bind` and `start`. Cancellation of
  a running guest is not yet a `#task` operation.
- **Child launches are synchronous and foreground.** Today a started child runs
  to completion on the controlling thread before `start`'s write returns;
  `exit` then reflects the finished status. There is no detached/background task
  supervisor.
- **`exit` is just text.** It is whatever the driver wrote — for qjs/wasm a
  numeric code, but the device imposes no schema.

These are the current edges, not permanent ones: persistent foreground
child-terminal ownership, cancellation, and richer session lifecycle are tracked
follow-ups.

## See also / next

- [Tasks own process identity](../concepts/tasks-own-process-identity) — why
  Wanix, not the engine, owns task ids and lifecycle.
- [Task drivers](../concepts/task-drivers) — how `noop`/`qjs`/`wasm`/`auto`
  dispatch works behind `start`.
- [The fd table](../concepts/the-fd-table) — what `bind ... fd/<n>` installs and
  why guest fds mirror into `#task/<id>/fd/<n>`.
- [The blocking-stream EOF contract](../concepts/blocking-stream-eof-contract) —
  the read/EOF semantics that `exit` and bound stream fds follow.
- Flow: [JS outside Chrome](../learn/js-outside-chrome) walks a qjs task end
  to end through these files.
