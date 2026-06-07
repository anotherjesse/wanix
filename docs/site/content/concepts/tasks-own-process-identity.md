---
title: Tasks Own Process Identity
slug: concepts/tasks-own-process-identity
pageType: concept
oneLiner: Wanix — not the engine — owns task id, parent, kind, cmd/env/cwd, exit status, namespace, and fds; QuickJS and wasm are just engines a task chooses.
audience: [developer]
tags: [tasks, process-model, shipped, cli]
sourceRefs:
  - crates/wanix-task/src/task.rs:16-211
  - crates/wanix-task/src/task/state.rs:10-22
  - crates/wanix-task/src/task/types.rs:6-26
  - crates/wanix-task/src/table.rs:12-197
  - crates/wanix-task/src/task_fs.rs:48-110
  - crates/wanix-task/src/task_files.rs:100-114
  - docs/adrs/0002-quickjs-wasi-task-runtime.md
  - AGENTS.md:133-138
seeAlso:
  - concepts/the-fd-table
  - devices/task
  - concepts/task-drivers
  - concepts/quickjs-snapshots-are-vm-images
  - concepts/wasmtime-as-substrate
prerequisites:
  - concepts/everything-is-a-file
usedInFlows: []
honestLimits:
  - "Snapshots are VM images, not task checkpoints: restore reattaches Wanix identity (namespace, cwd, argv/env, fds, exit observation) explicitly; it does not deserialize a saved Task."
  - "There are no hard CPU or memory limits on a task yet; bounded-execution knobs are deterministic-demo policy, not a sandbox for arbitrary untrusted code."
  - "Exec lives behind the local-trust boundary (#task is not exposed to untrusted public peers)."
---

# Tasks Own Process Identity

Wanix — not the engine — owns task id, parent, kind, cmd/env/cwd, exit status, namespace, and fds; QuickJS and wasm are just engines a task chooses.

A running program needs an identity: who am I, who started me, what command line, what working directory, what files am I holding open, and did I exit cleanly? In most runtimes that bookkeeping belongs to whatever interpreter or VM is executing the code, so swapping engines means rebuilding the process model. Wanix refuses that coupling. Identity is a Wanix-owned record; QuickJS and compiled wasm are interchangeable engines that a task hands its argv and fds to. This is what lets the same `#task` files, the same fd table, and the same exit semantics describe a JavaScript script and a compiled `.wasm` binary without either runtime defining what "process" means.

## Show it: drive a child task through files

Allocate a task, set its command, start it, and read its exit status — all as file plumbing over `#task`:

```sh
# Ask what engines you can start, then allocate one.
ls '#task/new'                  # -> auto noop qjs wasm ...
id=$(cat '#task/new/qjs')       # allocate a qjs task, get its id

echo 'examples/qjs-demo.js' > "#task/$id/cmd"
echo 'start' > "#task/$id/ctl"  # the control file kicks the driver
cat "#task/$id/exit"            # observable exit status, Wanix-owned
```

None of those reads or writes touch the QuickJS engine directly. `cmd`, `ctl`, and `exit` are fields of a Wanix record; the engine only runs once the driver's `start` fires (`crates/wanix-task/src/table.rs:122-154`). Swap `qjs` for `wasm` in the allocate step and every other line is identical, because the identity is the same shape regardless of engine. Plan 9 called this the process file system — `/proc` made a process inspectable and controllable as files. Wanix's `#task` is that idea applied to any WASI engine. See [the #task device](/devices/task).

## What a task actually is

A `Task` is a cheap handle: `Arc<Mutex<TaskState>>`, where cloning a `Task` points clones at the *same* state (`crates/wanix-task/src/task.rs:16-19`). All the identity lives in one struct (`crates/wanix-task/src/task/state.rs:10-22`):

```rust
pub(super) struct TaskState {
    pub(super) id: TaskId,
    pub(super) parent: Option<TaskId>,
    pub(super) kind: String,        // "qjs", "wasm", "noop", "auto", ...
    pub(super) spec: TaskSpec,      // explicit program/args/env/cwd
    pub(super) cmd: String,         // raw command text
    pub(super) cmd_argv: Option<Vec<String>>,
    pub(super) env: Vec<String>,    // KEY=value lines
    pub(super) dir: NormalizedPath, // working directory
    pub(super) exit: String,        // exit status text
    pub(super) namespace: Namespace, // this task's private file view
    pub(super) fds: FdTable,         // open file descriptors
}
```

Read that list against the engines. Nothing in it is QuickJS-shaped or Wasmtime-shaped. `id`/`parent`/`kind` are the genealogy; `cmd`/`cmd_argv`/`env`/`dir` are the launch knobs; `exit` is the result; `namespace` and `fds` are the two capability surfaces. The engine receives projections of these — argv/env/cwd flow into the guest, fd 0/1/2 back the guest's stdio — but never owns them (`docs/adrs/0002-quickjs-wasi-task-runtime.md`, Decision section). The accessor methods on `Task` (`id`, `parent_id`, `kind`, `cmd`, `env`, `dir`, `exit`, `namespace`, `bind`, and their setters) are the entire public surface (`crates/wanix-task/src/task.rs:43-194`), and each one just locks the state and reads or writes a field.

Two of those fields are themselves whole concepts elsewhere. The `namespace` is the task's private, per-process file view — two tasks can see different files at the same path. The `fds` are an explicit table that mirrors guest file descriptors into Wanix-observable state. They get their own pages: [the fd table](/concepts/the-fd-table) and [per-process namespaces](/concepts/per-process-namespaces).

## TaskId: a stable, non-zero handle

A `TaskId` wraps a `NonZeroU64`, and constructing one with `0` panics — root tasks start at `1` (`crates/wanix-task/src/task/types.rs:6-26`). Non-zero is a small but load-bearing choice: it makes "no task" representable as `Option<TaskId>` with zero runtime cost, which is exactly how `parent` is stored — a root task has `parent: None`, a child carries `Some(parent_id)`. The id is stable for the life of the table entry, so `#task/<id>` is a durable address and `#task/self` resolves to the current task's id (`crates/wanix-task/src/task_fs.rs:102-109`).

## Why the engine never owns identity

ADR 0001 fixes Wasmtime as the *execution substrate*, not the process model, and ADR 0002 draws the durable line: Wanix owns task identity, namespace, cwd, argv/env, stdio, fd tables, service files, exit state, and execution policy "for *any* WASI task runtime," while the runtime crate owns engine mechanics — instantiation, guest-memory decoding, fixture loading, snapshots (`docs/adrs/0002-quickjs-wasi-task-runtime.md`, Context and Decision). That is enforced structurally: `wanix-task` must not depend on `wanix-qjs`, `wanix-wasm`, or `wanix-wasi` (`AGENTS.md:133-138`). The dependency points the other way — a runtime crate is a task *driver* that plugs into the table. So adding a third engine is a `register_driver` call, not a rewrite of what a process is. See [task drivers](/concepts/task-drivers) and [Wasmtime as substrate](/concepts/wasmtime-as-substrate).

## Under the hood: the table, the driver map, the auto-bound view

The `TaskTable` is the registry that ties it together (`crates/wanix-task/src/table.rs:12-21`): a `next_id` counter, a `BTreeMap<TaskId, Task>`, and a `BTreeMap<String, Arc<dyn TaskDriver>>` mapping each kind to its driver. Allocation bumps `next_id`, mints the `TaskId`, and — this is the quiet part — binds a `#task` filesystem view scoped to the new id directly into that task's own namespace before the task is inserted (`crates/wanix-task/src/table.rs:182-194`). So every task can introspect itself through `#task/self` without anyone wiring it up after the fact; the identity is self-describing by construction.

`start` resolves the driver. If the kind is `auto`, the table walks the registered drivers and asks each one's `check` whether it claims this task (e.g. a `.wasm` cmd), sets the resolved kind, and calls `start`; otherwise it looks the kind up directly (`crates/wanix-task/src/table.rs:122-154`). The `#task/<id>` directory exposes the fields as files — `cmd`, `ctl`, `dir`, `env`, `exit`, `fd`, `id`, `kind` (`crates/wanix-task/src/task_files.rs:100-114`) — with `id` and `kind` read-only and `ctl` accepting `start`. That file set *is* the process-control API; there is no second one hiding behind it.

## See also

- [The fd table](/concepts/the-fd-table) — the open-handle half of task identity, mirrored from guest fds.
- [The #task device](/devices/task) — the file catalog (`new`, `<id>/cmd`, `ctl`, `exit`, `fd`, ...) that exposes this state.
- [Task drivers](/concepts/task-drivers) — how an engine registers as a `kind` and is selected by `check`/`start`.
- [QuickJS snapshots are VM images](/concepts/quickjs-snapshots-are-vm-images) — why a frozen engine is not a frozen task.
- [Wasmtime as substrate](/concepts/wasmtime-as-substrate) — the execution layer that never owns the process model.
- [Per-process namespaces](/concepts/per-process-namespaces) — the private file view carried in `TaskState`.

## Status / honest limits

- **Snapshots are VM images, not task checkpoints.** A QuickJS snapshot freezes engine state, not the Wanix `Task`. Restore must *explicitly reattach* the Wanix-owned identity — namespace, mounts, cwd, argv/env, stdio, task fds, live WASI providers, and exit observation (`docs/adrs/0002-quickjs-wasi-task-runtime.md`). There is no serialized `Task` format; identity is reconstructed on the Wanix side and the engine VM is reattached to it. Open dynamic descriptors block snapshot.
- **No hard resource limits yet.** QuickJS timers, promise-job budgets, interrupt callbacks, and heap limits are bounded host-execution *policy* for deterministic demos and tests — not a general scheduler, cancellation model, or sandbox. A task is cheap, scalable isolation, not a safe container for arbitrary untrusted code, and there are no enforced CPU/memory ceilings.
- **Exec is local-trust only.** The `#task` device is not exposed to untrusted public peers. Allocating and starting tasks is an operation on a node you control; the mesh does not yet grant remote exec to untrusted principals.
