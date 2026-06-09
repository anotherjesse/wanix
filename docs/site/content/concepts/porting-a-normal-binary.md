---
title: Porting a Normal Binary (What a Wanix Task Sees)
slug: concepts/porting-a-normal-binary
pageType: concept
oneLiner: A Wanix task is an ordinary WASI program — same open/read/write, same argv/env — so file-and-stdio software ports by recompiling; what changes is what the syscalls refer to, and async/socket/event-loop software is the part that needs rework.
audience: [developer]
tags: [task-runtime, wasm, wasi, porting, shipped, caveat]
sourceRefs:
  - crates/wanix-wasi-host/src/core.rs:34-39
  - crates/wanix-wasi/src/ctx/path.rs:12-18
  - crates/wanix-wasi/src/task_config.rs:24-45
  - crates/wanix-sh/src/ns.rs:54-104
  - crates/wanix-sh/src/exec.rs:1-9
  - crates/wanix-task/src/task.rs:189-210
seeAlso:
  - concepts/everything-is-a-file
  - concepts/wanix-backed-wasi
  - concepts/command-style-wasm-linker
  - concepts/host-not-ambient-authority
  - concepts/per-process-namespaces
  - concepts/task-exit-closes-fds
prerequisites:
  - concepts/everything-is-a-file
  - concepts/command-style-wasm-linker
usedInFlows: []
honestLimits:
  - poll_oneoff is ERRNO_NOSYS, so event-loop, epoll/select, and non-blocking-readiness software does not port cleanly — this is a command-style WASI subset, not a server-style host.
  - There are no host sockets and no ambient global namespace; a port that keeps its socket/IPC client libraries has nothing to bind to inside the sandbox.
  - Pipeline stages run concurrently on per-task host threads against the bounded `#pipe` (ADR 0010 tier 2); the shell guest itself stays single-threaded.
  - The toolchain story (cargo -> wasm32-wasip1 -> checked-in .wasm) is fixture-specific today; wanix-sh and a jaq guest are the only worked ports, so there is no general "port your CLI" workflow yet.
---

# Porting a Normal Binary (What a Wanix Task Sees)

A Wanix task is an ordinary WASI program — same `open`/`read`/`write`, same `argv`/`env` — so file-and-stdio software ports by recompiling; what changes is what the syscalls *refer to*, and async/socket/event-loop software is the part that needs rework.

The honest one-sentence answer to "how different is it to write a Wanix/Plan 9 binary versus a normal one" is: **Plan 9 doesn't change the syscalls, it changes the referents.** `open` still means `open` — it just resolves through a private per-task namespace where a "file" can be a device, a service, or another machine. Code that already thinks in files travels well. Code that thinks in sockets and event loops has to be rethought. This page draws the exact line so you can predict, before you start, whether a given program is a recompile or a rewrite.

## The 90% that is identical

A Wanix task is a `wasm32-wasi` (or QuickJS) program that calls `open`, `read`, `write`, `close`, reads `argv` and `environ`, owns fds 0/1/2, and returns an exit code. Nothing about authoring it is Wanix-specific. The shell is the proof: `wanix-sh` is a normal Rust program compiled to `wasm32-wasip1` using the off-the-shelf `brush-parser` crate, and its *entire* operating-system surface is one trait, `NamespaceOps`, that bottoms out in ordinary file calls (`crates/wanix-sh/src/ns.rs:54-104`). A jq clone (`jaq`) compiles unmodified and runs as a task.

So for the common case — a filter, a compiler, a tool that reads files and writes files — you write a normal Unix-ish program and it works. You do **not** have to think about namespaces. The parent constructs the child's namespace; the child just sees "a filesystem," the way every program always has. This is Plan 9's original bet: *the* interface is the filesystem, and you already know how to use a filesystem.

## The four things that are actually different

The differences are not in the shape of the API. They are four shifts in what the API *means*.

### 1. "Everything is a file" is literal, so non-file I/O becomes file I/O

On Linux you reach a key/value store through a socket plus a client library, a pub/sub bus through a broker SDK, another machine through gRPC. In Wanix all of those are `open`:

```text
#kv/<key>            a key/value cell
#plumb/<topic>/send  publish a message
/n/peerA/#kv/<key>   a peer's key/value cell, across the mesh
```

The *idiomatic* port deletes the client libraries and just does file I/O. A literal port keeps its sockets and SDKs — and they have nothing to bind to, because there are no host sockets in the sandbox. So porting isn't harder, but the idiomatic rewrite asks you to *throw away* abstractions, which is a mindset flip. The leverage (a peer's device is `/n/peerA/#kv` with zero per-service network code) only appears if you take it. See [everything is a file](/concepts/everything-is-a-file) and [devices import for free](/concepts/devices-import-for-free).

### 2. There is no ambient global namespace — names are per-task

On Unix `/etc/passwd` means the same thing to every process. In Wanix a task only sees what was bound into *its* namespace. Even the service devices resolve from the namespace root only because they were bound there — `#task`, `#term`, `#kv`, and friends are a fixed allow-list that bypass cwd, while every other path is relative like normal (`crates/wanix-wasi/src/ctx/path.rs:12-18`):

```rust
const ROOTED_SERVICE_DEVICES: &[&str] = &[
    "#task", "#term", "#kv", "#pipe", "#plumb", "#cas", "#agent", "#mesh", "#cpu",
];
```

For a **leaf** program this is invisible. For a **parent** — a shell, a supervisor, anything that spawns — it is the central fact: launching a child is not `fork`/`exec` with inherited everything. It is file-based orchestration (`open #task/new/<kind>` → write `cmd`/`env`/`dir` → `ctl bind <src> fd/<n>` → write `start`), and the child gets exactly the namespace you handed it, nothing more. **Writing leaf programs is easy; writing things that spawn and compose is where you think Plan 9.** That is precisely the cut in the original question — if you don't want to think about child namespaces, you are writing a leaf, and it stays simple. See [per-process namespaces](/concepts/per-process-namespaces) and [host files are not ambient authority](/concepts/host-not-ambient-authority).

### 3. No async, no readiness model

This is the one genuinely sharp porting edge. `poll_oneoff` — the WASI call a program uses to wait until an fd is ready — is wired to "not implemented" (`crates/wanix-wasi-host/src/core.rs:34-39`):

```rust
linker.func_wrap(m, "poll_oneoff",
    |_, _i, _o, _n, _ne| ERRNO_NOSYS)?;
```

Tasks run `_start` to completion on one thread. Synchronous, batch-shaped tools port trivially. Anything built on an event loop, `epoll`/`select`, non-blocking sockets, or "wait on N fds at once" does **not** map cleanly and needs rework. Per-task threads exist for commands (ADR 0010 tier 2): `a | b` pipelines run their stages concurrently on detached task threads against the bounded `#pipe` (`crates/wanix-sh/src/pipeline.rs`), but a single guest still gets exactly one thread and no readiness multiplexing beyond `fd_read` `poll_oneoff`. See [the command-style wasm linker](/concepts/command-style-wasm-linker).

### 4. fds are explicit capabilities, not ambient inheritance

A child gets fds 0/1/2 because the parent *bound* them — fds 0/1/2 are wired as proxies into the task's own fd table, and dynamically opened fds mirror into it too (`crates/wanix-wasi/src/task_config.rs:24-45`). It is close enough to Unix fd semantics that ported code never notices. The model underneath is "default-deny, parent grants explicitly," which is what makes the same unmodified binary safe to run against a confined or remote namespace. A task dropping its fds on exit is also what makes pipe EOF work (`crates/wanix-task/src/task.rs:189-210`); see [task exit closes fds](/concepts/task-exit-closes-fds).

## The verdict, by software shape

Sort the program you want to port into one of two buckets:

- **Synchronous, file-and-stdio shaped** (compilers, linters, formatters, filters, `jq`, most CLI tools): *easier than a normal port.* Often just a `wasm32-wasip1` recompile. It calls `open`/`read`/`write`, and those resolve through the namespace transparently.
- **Async / event-loop / socket-server shaped** (anything on `epoll`/`select`, non-blocking I/O, a network listener, a long-lived reactor): *harder.* You are fighting the missing async model (`poll_oneoff` = NOSYS) and the absence of host sockets — not the Plan 9 model. Expect to restructure to a synchronous, run-to-completion command, and to replace network/IPC with file I/O against service devices.

One caveat on that second bucket: the no-async limit is *current implementation state*, not inherent to Plan 9. Pipelines already run concurrently on per-task threads (ADR 0010 tier 2), so "async software is harder to port" keeps softening as the host grows readiness primitives.

## See also

- [Everything is a file](/concepts/everything-is-a-file) — why storage, tasks, terminals, and peers are all `FileSystem`s you `open`.
- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — the live context every fd and path call resolves through, instead of the host OS.
- [Command-style wasm linker](/concepts/command-style-wasm-linker) — the exact import subset a guest can rely on, and why `poll_oneoff` is NOSYS.
- [Per-process namespaces](/concepts/per-process-namespaces) — the private, parent-supplied file view that replaces the ambient global one.
- [Host files are not ambient authority](/concepts/host-not-ambient-authority) — how a parent hands a leaf exactly one directory and no more.
- [Task exit closes fds](/concepts/task-exit-closes-fds) — the fd-drop-on-exit primitive that makes pipe EOF (and honest pipelines) work.

## Status / honest limits

- **`poll_oneoff` is `ERRNO_NOSYS`.** Event-loop, `epoll`/`select`, and non-blocking-readiness software does not port cleanly; it must be restructured into a run-to-completion command. This is a command-style WASI subset, not a server-style host (`crates/wanix-wasi-host/src/core.rs:34-39`).
- **No host sockets, no ambient namespace.** A port that keeps its socket or IPC client libraries has nothing to bind to inside the sandbox; the idiomatic move is to replace them with file I/O against service devices. Names are per-task and parent-supplied — there is no global `/etc` every task shares.
- **Pipelines are concurrent and bounded.** Stages run on their own host threads against the 64 KiB `#pipe` (Plan 9's concurrent + bounded model, ADR 0010 tier 2); a producer is back-pressured by its consumer instead of buffering everything (`crates/wanix-sh/src/pipeline.rs`).
- **No general porting workflow yet.** The cargo → `wasm32-wasip1` → checked-in `.wasm` toolchain is fixture-specific; `wanix-sh` and a `jaq` guest are the only worked ports. This page is the conceptual map, not a copy-paste recipe — that waits on a second real port to generalize from.
