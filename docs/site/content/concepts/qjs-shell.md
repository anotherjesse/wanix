---
title: qjs-shell (Shell as a Guest Program)
slug: concepts/qjs-shell
pageType: concept
oneLiner: The bundled interactive shell is JavaScript inside a Wanix task driving #term and #task service files — not a separate process model.
audience: [newcomer, developer]
tags: [terminal, shell, qjs, task, cli, shipped, caveat]
sourceRefs:
  - examples/qjs-term-shell-demo.js:1-962
  - crates/wanix-cli/src/qjs_term.rs:11-32
  - crates/wanix-cli/src/qjs_term/session.rs:22-213
  - docs/adrs/0003-terminal-device-and-shell-lifecycle.md:59-68
seeAlso:
  - devices/term
  - concepts/qjs-task
  - concepts/raw-vs-cooked-input
  - concepts/resize-winch-lifecycle
  - devices/task
prerequisites:
  - devices/term
  - concepts/qjs-task
usedInFlows: []
honestLimits:
  - No pipes and no job control: a line runs one command, not a pipeline.
  - The only external program launch is a synchronous child task (#task/new/auto — a .js or .wasm program, dispatched by extension); there is no general exec, and a .wasm child gets launch + wait + exit status but no interactive terminal handoff.
  - cd cannot enter a #-device path; service paths are not navigable directories.
  - Bounded event-loop pumping around a session is host lifecycle policy, not a Wanix scheduler, signal, or cancellation model.
---

# qjs-shell (Shell as a Guest Program)

The bundled interactive shell is JavaScript inside a Wanix task driving `#term` and `#task` service files — not a separate process model.

Run `wanix qjs-shell` and you get a `$ ` prompt, `ls`, `cd`, `cat`, and the ability to launch other programs. It looks like a tiny `bash`. It is not. The whole thing is a single QuickJS script — `examples/qjs-term-shell-demo.js` — running as one ordinary Wanix `qjs` task. Every "shell feature" you see is that script opening, reading, and writing files: the terminal it talks through is `#term/<id>/data`, its current directory is `#task/self/dir`, and the children it spawns are allocated from `#task/new/auto`. There is no shell runtime in Rust. There is a guest program and the same service devices every other task can reach.

## Show: the prompt is a script reading its own files

Start a session and the first line printed is the task's own id, read straight out of its namespace (`examples/qjs-term-shell-demo.js:943`):

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

wanix qjs-shell
# shell task: 1
# $ ls
```

The script's startup is three file reads and a callback registration. It loads `#task/self/dir` to seed its working directory (`examples/qjs-term-shell-demo.js:14`), opens `#term/<id>/winch` to learn the window size (`examples/qjs-term-shell-demo.js:929`), and registers `os.setReadHandler(0, ...)` so that whenever terminal input is readable on fd 0 it drains a resize update and then reads bytes to feed the line parser (`examples/qjs-term-shell-demo.js:946-959`). That is the entire engine: a read handler on stdin, plus a prompt printed to stdout. The "shell" is an event loop over `#term`.

## The builtins are file operations

Type a command and the script splits it into words, then dispatches on the first word (`examples/qjs-term-shell-demo.js:724-860`). Every builtin is a thin wrapper over `qjs:os` file calls against the namespace:

- Filesystem: `cd`, `ls`, `cat`, `write`, `mkdir`, `rm`, `rmdir`, `mv`, `cp`, `ln -s`, `readlink`, `stat`, `lstat` — each resolves the argument against the current directory and calls `os.readdir`, `os.mkdir`, `os.rename`, `os.symlink`, `os.stat`, and friends (`examples/qjs-term-shell-demo.js:383-657`).
- Task environment: `env`, `setenv`, `unsetenv` read and rewrite `#task/self/env` line by line (`examples/qjs-term-shell-demo.js:254-369`).
- Inspection: `ps` lists `#task` and prints each task's `kind`, `exit`, `dir`, and `cmd` fields (`examples/qjs-term-shell-demo.js:268-292`); `id` prints `#task/self/id`; `pwd`, `status`, `size`, `echo`, `later`, and `exit` round out the set (`examples/qjs-term-shell-demo.js:737-857`).

`cd` is the clearest example of "file operation, not process state." It validates the target with `os.readdir`, then writes the new path to `#task/self/dir` (`examples/qjs-term-shell-demo.js:409`). The shell's working directory is a service file, so `ps` in another tool sees the same `dir` the shell navigated to. Per ADR 0003, `#task/self/dir` is the logical shell cwd while the WASI root preopen stays at the namespace root, which is exactly what lets `cd` walk the served tree without re-rooting the task (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:62-64`).

A guardrail falls out of the file model: `cd`, `write`, `rm`, `mv`, and the rest refuse a `#`-prefixed argument with a message like "service paths are not directories" (`examples/qjs-term-shell-demo.js:398-399`). The devices are reachable by their own command-specific helpers, not by generic shell mutation.

## Launching a child: #task/new/auto, then bind and start

The one command that runs another program is `qjs`. It does not fork or exec — it drives the task device by hand (`examples/qjs-term-shell-demo.js:659-722`). Reading `#task/new/auto` allocates a fresh task id (the host task table picks the driver from the program extension, so a `.js` and a `.wasm` child launch the same way — ADR 0002's both-first-class contract); the shell then writes the child's `cmd`, copies its own `env`, sets the child's `dir`, and binds fds 0/1/2 before writing `start`:

```sh
$ qjs child.js alpha 'two words' < input.txt > out.txt 2> err.txt
```

That single line expands into a sequence of writes to `#task/<child>/ctl`. With no redirection the shell binds the child's fd 0/1/2 to its *own* fd 0/1/2 — `bind #task/<parent>/fd/0 fd/0` — so the child shares the terminal (`examples/qjs-term-shell-demo.js:687-707`). With `<`, `>`, or `2>` it binds the named namespace file instead, creating an empty output file first when the target is a real path (`examples/qjs-term-shell-demo.js:692-704`). After `start`, the shell reads `#task/<child>/exit`, which blocks until the child finishes, and records it as `lastStatus` so `status` can report it (`examples/qjs-term-shell-demo.js:708-716`). When the child reads from the terminal (no `<` given), the shell hands off any bytes typed on the same input line so foreground stdin works (`examples/qjs-term-shell-demo.js:862-869`, `241-252`).

This is the Plan 9 move named at last: a shell is just a program that composes other programs by manipulating their file namespaces. The `#task` device is the process table and the spawn primitive at once.

## Served sessions: pump turns, release the terminal

The native `qjs-shell` reads your real terminal, but the same script runs behind a browser cockpit terminal through `QjsShellSession` (`crates/wanix-cli/src/qjs_term/session.rs:22-49`). The session allocates a `qjs` task, binds the served root and the bundled shell source into its namespace, attaches a `#term` resource, and evaluates the script. Input arrives as `session.input(bytes)`, which feeds the terminal, runs a fixed number of ready-IO turns, and drains output (`crates/wanix-cli/src/qjs_term/session.rs:63-71`). Delayed output — the `later` builtin's timer — surfaces through `session.pump()` even with no new input (`crates/wanix-cli/src/qjs_term/session.rs:87-97`).

The lifecycle boundary matters: when the session is dropped or closed it writes `close` to `#term/<id>/ctl` exactly once, releasing the terminal resource it owns (`crates/wanix-cli/src/qjs_term/session.rs:120-145`). Per ADR 0003 that `close` is a resource-cleanup signal, not task cancellation or a process-group signal (`docs/adrs/0003-terminal-device-and-shell-lifecycle.md:65-68`). Wanix keeps owning task identity, fds, and exit state across both the native and served paths; the shell session only owns the terminal.

## See also

- [#term device](/devices/term) — the `new`/`<id>/data`/`<id>/program`/`<id>/winch` contract the shell drives.
- [The qjs task](/concepts/qjs-task) — what the shell *is*: one QuickJS program running as a Wanix task.
- [Raw vs cooked input](/concepts/raw-vs-cooked-input) — who owns echo, backspace, Ctrl-C, and Ctrl-D when the shell runs raw.
- [Resize / winch lifecycle](/concepts/resize-winch-lifecycle) — how `#term/<id>/winch` deliveries reach the shell's `size` builtin.
- [#task device](/devices/task) — the process table and spawn primitive the `qjs` builtin manipulates.

## Status / honest limits

The bundled shell is deliberately small, and its boundaries are file-model boundaries, not missing polish:

- **No pipes, no job control.** A line dispatches exactly one command; there is no `|`, no `&`, and no background-job table. Redirection is the only composition primitive, and only for the `qjs` builtin (`examples/qjs-term-shell-demo.js:202-239`).
- **The only external program is a synchronous child task.** `qjs PROGRAM(.js|.wasm)` is the whole exec story; there is no general program execution beyond allocating a child through `#task/new/auto` (`examples/qjs-term-shell-demo.js:659-722`). A `#`-device path is rejected as "not an executable script," and a `.wasm` child gets launch + wait + exit status only — no interactive terminal handoff (redirect its stdin, or don't read it).
- **Service paths are not navigable.** `cd` into a `#`-prefixed path fails by design; the devices are reachable by name through their own helpers, not as directories (`examples/qjs-term-shell-demo.js:398-417`).
- **Pumping is host policy, not a kernel.** The bounded ready-IO turns and idle event-loop budget around a `QjsShellSession` are lifecycle policy for an already-evaluated runtime (`crates/wanix-cli/src/qjs_term/session.rs:87-97`). They are not a Wanix scheduler, signal system, or cancellation model — Ctrl-C clears the input line and Ctrl-D exits the shell, but neither signals a running child (`examples/qjs-term-shell-demo.js:871-894`).
