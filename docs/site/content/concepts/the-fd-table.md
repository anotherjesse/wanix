---
title: The fd Table
slug: concepts/the-fd-table
pageType: concept
oneLiner: Each task has an fd table reserving 0/1/2 for stdio and allocating dynamic fds from 3, holding shared open-file handles that survive a closing source task.
audience: [developer]
tags: [tasks, fds, wasi, shipped]
sourceRefs:
  - crates/wanix-task/src/fd.rs:128-231
  - crates/wanix-task/src/fd.rs:32-55
  - crates/wanix-task/src/task_fs.rs:71-78
  - rust-walkthrough.md:180-209
  - docs/adrs/0002-quickjs-wasi-task-runtime.md:54-59
seeAlso: [concepts/tasks-own-process-identity, concepts/shared-wasi-fd-contract, devices/task, concepts/qjs-task]
prerequisites: [concepts/tasks-own-process-identity]
usedInFlows: []
honestLimits:
  - fd mirroring is one shared contract through the wanix-wasi config path, not reimplemented per runtime.
  - Only dynamic regular-file fds are mirrored into the Wanix table; directory fds stay WASI-internal.
  - The fd table is not itself locked across calls — it holds per-file Arc<Mutex<...>> handles, so do not hold the table while calling another filesystem.
---

# The fd Table

Each task has an fd table reserving 0/1/2 for stdio and allocating dynamic fds from 3, holding shared open-file handles that survive a closing source task.

Open a file inside a Wanix task and you get back a small integer. That integer is an index into the task's *fd table* — `FdTable` in `crates/wanix-task/src/fd.rs:128`. The table is per-task state, the same way the namespace is per-task state: it is part of what makes a task a process rather than a bare function call. This page is about what those numbers point at, why a handle keeps working after the task that opened it is gone, and how the WASI runtimes stay in sync with it.

## The one-liner, demonstrated

Run the fd demo from the walkthrough:

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'
wanix qjs examples/qjs-fd-demo.js
```

```text
read fd: 4
saw fd API: true
write fd: 5
bytes: 21
hello from a Wanix fd
```

The first opened file came back as fd `4`, the second as fd `5` (`rust-walkthrough.md:180-209`). Nothing in the guest asked for those numbers; the table chose them. That is the allocation rule, and it is the first thing `FdTable::new` sets: `next_fd` starts at `3` (`crates/wanix-task/src/fd.rs:145-152`). Descriptors 0, 1, and 2 are *reserved* — `Fd::STDIN`, `Fd::STDOUT`, `Fd::STDERR` (`crates/wanix-task/src/fd.rs:11-18`) — and `open` always hands out from 3 upward, one at a time:

```rust
pub fn open(&mut self, file: Box<dyn File>, path: NormalizedPath) -> Fd {
    let fd = Fd::new(self.next_fd);
    self.next_fd += 1;
    self.files.insert(fd, OpenFile::new(file, path));
    fd
}
```

`insert_at` and `insert_at_if_vacant` let a driver place stdio at a fixed slot — bind fd 1 to a terminal's `program` file, say — and they bump `next_fd` past whatever they installed so a later `open` never collides (`crates/wanix-task/src/fd.rs:162-180`). This is the same reservation a Unix process makes; Wanix just owns the table explicitly instead of inheriting it from the kernel.

## An fd entry is a shared handle, not a copy

What lands in the table is an `OpenFile`, and its shape is the whole point:

```rust
pub struct OpenFile {
    file: Arc<Mutex<Box<dyn File>>>,
    path: NormalizedPath,
}
```

(`crates/wanix-task/src/fd.rs:32-55`.) The open `File` lives behind an `Arc<Mutex<...>>`, and `OpenFile` is `Clone`. So `FdTable::file` does not copy the file — it clones the `Arc`, handing back another reference to the *same* underlying handle (`crates/wanix-task/src/fd.rs:221-224`). Reads, writes, and seeks all take the lock and go through to the one shared file (`crates/wanix-task/src/fd.rs:57-79`), which means the byte offset is shared too: two holders of the same fd entry advance one cursor.

This is `dup`/share semantics expressed in Rust ownership terms. A descriptor in the table is a reference-counted bind to a live file object, not a snapshot of its bytes.

## Why a handoff outlives the task that opened it

The shared-handle design pays off the moment a service file changes hands. Picture one task opening a `#pipe` read end, then handing that descriptor to a child and exiting. In a copy-the-handle world the child would be left holding a dangling reference once the parent's table dropped. Here, the child's entry is an `Arc` clone of the same `OpenFile`. When the source task's table drops — `close` removes the entry, and dropping the table drops its entries (`crates/wanix-task/src/fd.rs:182-189`) — the `Arc` strong count merely decrements. The file stays alive as long as *any* holder keeps a clone.

So a capability you opened (a pipe, a CAS blob, an agent's `events` stream) remains readable through a descriptor you passed on, even after the task that first opened it is finished. The fd table makes the open handle the unit of lifetime, decoupled from the task that created it. That is exactly the behavior a Plan 9 file descriptor has, and it is what lets one task set up plumbing and another consume it.

## Under the hood: which fds the WASI runtimes mirror

A QuickJS or `.wasm` guest runs inside Wasmtime with its own WASI fd numbering. Wanix does not throw that away — it *mirrors* the fds that matter back into this table. The rule (ADR 0002, `docs/adrs/0002-quickjs-wasi-task-runtime.md:54-59`): when a WASI task exposes a guest fd as Wanix-observable state, the adapter mirrors that fd through the task fd table *at the same number* and releases it when the guest closes it. That is why the demo's namespace-level fd `4` is also the guest's fd `4`.

Two boundaries keep this honest:

- **Only dynamic regular-file fds are mirrored.** Directory fds may stay WASI-internal until a Wanix task fd contract actually needs them (`docs/adrs/0002-quickjs-wasi-task-runtime.md:56-57`). The table is for byte files you might hand off or inspect, not for every internal WASI handle.
- **One contract, both runtimes.** The wasm driver builds its live WASI config through the same `wanix-wasi` task-config path as qjs, so fd mirroring is shared, not reimplemented per runtime (`docs/adrs/0002-quickjs-wasi-task-runtime.md:57-59`). See [the shared WASI fd contract](/concepts/shared-wasi-fd-contract).

Because the mirror lands in this table, the `#task` device can surface it. `#task/self/fd` lists the live fd numbers, one directory entry per open descriptor (`crates/wanix-task/src/task_fs.rs:71-78`). The fd table is private per-task state, but it is also — like everything else — readable as files. See [tasks own process identity](/concepts/tasks-own-process-identity) for the rest of `#task/self`.

## See also

- [Tasks own process identity](/concepts/tasks-own-process-identity) — the task's id, cmd, env, and exit status, all as files.
- [The shared WASI fd contract](/concepts/shared-wasi-fd-contract) — how qjs and wasm guests both mirror fds through one config path.
- [The #task device](/devices/task) — `#task/new`, `#task/self`, and the `fd/<n>` listing this table backs.
- [The qjs task](/concepts/qjs-task) — the QuickJS runtime whose `qjs:os` fd API the demo exercises.

## Status / honest limits

- **Mirroring is selective, by design.** Only dynamic regular-file fds reach this table; directory fds remain WASI-internal until a contract needs them (`docs/adrs/0002-quickjs-wasi-task-runtime.md:54-57`). Do not expect every internal guest handle to appear in `#task/self/fd`.
- **The contract is shared, not duplicated.** Both WASI runtimes mirror through the one `wanix-wasi` task-config path (`docs/adrs/0002-quickjs-wasi-task-runtime.md:57-59`); there is no per-runtime fd bookkeeping to drift apart.
- **Per-file locking, not table locking.** Each entry carries its own `Arc<Mutex<Box<dyn File>>>` (`crates/wanix-task/src/fd.rs:34`). Reads and writes lock only that file. Keep to the project rule: never hold a namespace or filesystem lock while calling into another filesystem, and the same caution applies to fd handles.
