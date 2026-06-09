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
  - "The wasm and qjs task drivers release fds on exit (crates/wanix-wasm/src/driver.rs, crates/wanix-qjs/src/driver.rs), and a detached start releases them even when no driver ran; any new driver must adopt the same close on completion, as piped commands need it."
  - "Pipeline stages now run CONCURRENTLY against the bounded blocking #pipe (ADR 0010 tier 2): each external stage runs on its own host thread (detached `start &`), so a producer blocks once 64 KiB is buffered until the consumer drains. A write after the last reader closes is a broken-pipe error, not a hang."
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

The Wanix-native shell (`wanix-sh`) runs as an ordinary wasm task and launches non-builtin commands as child tasks through the `#task` device (`crates/wanix-sh/src/exec.rs:48-58`, `crates/wanix-sh/src/ns.rs`). Because a finished child releases its fds, a producer that writes into a `#pipe` and exits leaves a closed write end behind it — so the consumer stage drains the buffer and sees a clean EOF. That is the whole mechanism behind an honest pipeline, and it is the literal Plan 9 model: the kernel reclaims a process's descriptors on exit, and the shell closes its own copies of the pipe ends.

- **External producer:** the shell binds the pipe *directly into the child* via `#task/<child>/ctl`, so the shell never holds a writer of its own — there is nothing for it to close. The child's exit drops the only writer.
- **Builtin producer:** the shell opens the pipe writer itself, writes the builtin's output, and drops its own handle — the Plan 9 "shell closes its ends" move.

## Concurrency: the Plan 9 model, restored

Plan 9 runs the stages of a pipeline **concurrently** against a **bounded** blocking pipe; `b` reads while `a` is still producing. Wanix now does the same (ADR 0010 tier 2): the shell launches every external stage with a detached `#task` start (`ctl` `start &`, one host OS thread per running command), the `#pipe` buffers 64 KiB and then blocks the producer until the consumer drains, and the shell collects exit statuses in stage order through the blocking `#task/<id>/wait` file. A producer whose consumer dies mid-stream gets a broken-pipe error instead of blocking forever.

The shell guest itself stays single-threaded: builtins run in-shell between launch and wait, and two adjacent builtins exchange bytes through shell memory rather than a pipe the shell could deadlock itself on.
