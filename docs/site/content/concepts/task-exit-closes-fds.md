---
title: A Task Exit Closes Its fds (and How Pipelines Get EOF)
slug: concepts/task-exit-closes-fds
pageType: concept
oneLiner: A finished task releases its whole fd table (Unix/Plan 9: an exited process holds no descriptors), which is what drops a pipeline producer's #pipe writer so the consumer observes EOF.
audience: [developer]
tags: [tasks, fds, shell, pipelines, plan9, in-progress]
sourceRefs:
  - crates/wanix-task/src/task.rs:188-213
  - crates/wanix-wasm/src/driver.rs:34-48
  - crates/wanix-wasi/src/task_config.rs:20-46
  - crates/wanix-pipe/src/channel.rs:71-104
  - crates/wanix-sh/src/exec.rs:48-58
  - crates/wanix-sh/src/ns.rs:11-67
seeAlso:
  - concepts/the-fd-table
  - concepts/tasks-own-process-identity
  - concepts/blocking-stream-eof-contract
  - devices/pipe
  - devices/task
  - concepts/qjs-shell
prerequisites:
  - concepts/the-fd-table
  - concepts/blocking-stream-eof-contract
usedInFlows: []
honestLimits:
  - "Only the compiled-wasm task driver releases fds on exit today (crates/wanix-wasm/src/driver.rs); other drivers (e.g. qjs) should adopt the same close on completion as piped commands need it."
  - "Pipeline stages run SEQUENTIALLY, not concurrently: stage a fully completes and buffers its whole output into the unbounded in-memory #pipe before stage b drains it. This is a deliberate divergence from Plan 9 (concurrent stages + a bounded blocking pipe) and must be revisited once tasks can run on their own threads."
  - "set_exit only records the status string and is callable mid-operation; it deliberately does NOT touch fds. Fd release is a separate step the driver performs when the run truly finishes."
---

# A Task Exit Closes Its fds (and How Pipelines Get EOF)

A finished task releases its whole fd table (Unix/Plan 9: an exited process holds no descriptors), which is what drops a pipeline producer's `#pipe` writer so the consumer observes EOF.

[The fd Table](the-fd-table) told you that an open-file handle *survives a closing source task* — bind a file into a child and the child keeps reading it even after the opener is gone. This page is about the other direction: what happens to the descriptors a task holds *in its own table* when that task itself finishes. The short answer is the Unix and Plan 9 answer: they all close. That single rule is what makes a shell pipeline terminate instead of hanging forever.

## Why a pipeline needs it

A `#pipe` read **blocks until bytes arrive or every writer is gone** (`crates/wanix-pipe/src/channel.rs:87` — the channel reports EOF only once `writers == 0`). See [the blocking-stream EOF contract](blocking-stream-eof-contract). So for `producer | consumer` to ever finish, the producer's *write end* of the pipe must actually be released when the producer is done. Nothing else will tell the consumer "no more bytes are coming."

Here is the trap. A task's stdio is wired by `task_wasi_config` as `TaskFdFile` proxies installed into the **task's own fd table** (`crates/wanix-wasi/src/task_config.rs:34-36`), and **tasks persist in the task table after they exit**. So if a producer binds its fd 1 to `#pipe/<id>/data` and then exits, the pipe's write end is *still sitting in the dead task's fd table*. `writers` never drops to zero. The consumer's read blocks forever.

## The rule: exit releases the fd table

The fix is to make a task behave like a process: when it finishes, it holds nothing. The compiled-wasm driver does exactly this — after the guest's run completes (success or trap), it calls `Task::close_all_fds()` before recording the exit code:

```rust
fn start(&self, task: &Task) -> FsResult<()> {
    let result = run_wasm_task(task);
    // A finished task releases its fds (Unix/Plan 9: an exited process holds
    // no descriptors), so a pipeline producer's `#pipe` writer drops and the
    // consumer observes EOF.
    task.close_all_fds();
    match result {
        Ok(code) => task.set_exit(code.to_string()),
        Err(err) => { let _ = task.set_exit("1"); Err(err) }
    }
}
```

`close_all_fds` is deliberately **separate from `set_exit`** (`crates/wanix-task/src/task.rs:188-213`). `set_exit` only records the status string, and it is callable *mid-operation* — a service file can set a task's exit as a side effect of a read while that read is still using an fd. Tearing the fd table down inside `set_exit` would yank descriptors out from under live operations. So fd release is its own step, performed by the driver at the one moment the run is genuinely over.

Anything that needs a task's *output* after it exits reads the fd's **backing** — the sink file, the pipe, the shared filesystem — not the task's fd table, which is now empty.

## How the shell uses it

The Wanix-native shell (`wanix-sh`) runs as an ordinary wasm task and launches non-builtin commands as child tasks through the `#task` device (`crates/wanix-sh/src/exec.rs:48-58`, `crates/wanix-sh/src/ns.rs`). Because child launch is synchronous and a finished child releases its fds, a producer that writes into a `#pipe` and exits leaves a closed write end behind it — so the next stage drains the buffer and sees a clean EOF. That is the whole mechanism behind an honest pipeline, and it is the literal Plan 9 model: the kernel reclaims a process's descriptors on exit, and the shell closes its own copies of the pipe ends.

- **External producer:** the shell binds the pipe *directly into the child* via `#task/<child>/ctl`, so the shell never holds a writer of its own — there is nothing for it to close. The child's exit drops the only writer.
- **Builtin producer:** the shell opens the pipe writer itself, writes the builtin's output, and drops its own handle — the Plan 9 "shell closes its ends" move.

## Where we diverge from Plan 9 (on purpose, for now)

Plan 9 runs the stages of a pipeline **concurrently** against a **bounded** blocking pipe; `b` reads while `a` is still producing. Wanix does not have per-task threads yet (that arrives with the interactive-shell work), so today pipeline stages run **sequentially** against the **unbounded** in-memory `#pipe`: stage `a` runs to completion and buffers its entire output, then stage `b` drains it. For finite output the result is identical; the difference is memory (a huge intermediate buffers in full) and the loss of producer/consumer overlap.

This is a known, deliberate divergence — the kind you take to ship the correct *result* before the correct *concurrency*. It should be revisited as soon as tasks can run on their own threads, restoring concurrent stages and a bounded pipe. Until then, treat large-output pipelines as buffering, not streaming.
