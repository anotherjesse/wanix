---
title: wanix-sh — a Wanix-native shell as a wasm task
slug: concepts/wanix-sh
pageType: concept
oneLiner: A Rust shell that uses brush-parser for bash syntax and a Wanix-native executor, compiled to wasm32-wasip1 and run as an ordinary #task — parse → lower → execute against one NamespaceOps seam.
audience: [developer]
tags: [shell, wasm, tasks, pipelines, in-progress]
sourceRefs:
  - crates/wanix-sh/src/lib.rs
  - crates/wanix-sh/src/lower.rs
  - crates/wanix-sh/src/exec.rs
  - crates/wanix-sh/src/ns.rs
  - crates/wanix-sh/src/expand.rs
  - crates/wanix-wasm/fixtures/shell-src/src/main.rs
seeAlso:
  - concepts/shell-command-resolution
  - concepts/task-exit-closes-fds
  - concepts/qjs-shell
  - concepts/compiled-wasm-task-driver
  - devices/task
  - devices/pipe
prerequisites:
  - concepts/compiled-wasm-task-driver
  - concepts/the-fd-table
usedInFlows: []
honestLimits:
  - "Interactive editing is byte-wise ASCII: echo, backspace, Ctrl-C line cancel, Ctrl-D exit. NO tab completion, NO history, NO cursor movement or arrow keys yet (escape sequences are ignored, not interpreted)."
  - "Pipeline stages launch one at a time from the single-threaded shell guest; once launched, external stages run CONCURRENTLY on their own host threads against the bounded #pipe (ADR 0010 tier 2), and adjacent builtins exchange bytes through shell memory — see concepts/task-exit-closes-fds."
  - "cwd is shell-LOCAL: cd/pwd/$PWD track it, but spawned children do not inherit a moved cwd (they run at root). Exported env DOES propagate to children."
  - "Honest-scope: control flow (if/for/while/case), functions, command/arithmetic substitution, ${VAR:-default}, globbing, tilde, fd-dup and here-doc redirects, and external `>>` append are recognized but return a clear `... not supported yet` error — never a silent no-op."
---

# wanix-sh — a Wanix-native shell as a wasm task

A Rust shell that uses [`brush-parser`](https://crates.io/crates/brush-parser) for bash syntax and a Wanix-native executor, compiled to `wasm32-wasip1` and run as an ordinary `#task` — parse → lower → execute against one `NamespaceOps` seam.

The [QuickJS shell](qjs-shell) proved a shell can run as a guest task driving `#term`/`#task` over the namespace. `wanix-sh` is the Rust successor: the shell is *just another program*, a compiled wasm command, not a special runtime. It reuses everything the [compiled-wasm task driver](compiled-wasm-task-driver) already gives a guest — namespace, fds, env, exit — and adds a small, honest shell language on top.

## Show: a shell pipeline through real tasks

```sh
wanix-rust wasm crates/wanix-wasm/fixtures/shell.wasm -c "echo '[1,2,3]' | jaq 'map(.+1)'"
```

```text
[2,3,4]
```

`echo` is a builtin; `jaq` is an *external command* resolved from a `bin` directory and run as its own `#task`, fed by a real [`#pipe`](devices/pipe). No part of the shell knows what `jaq` is — it is [resolved like any command](shell-command-resolution).

## The three stages

1. **parse** (`syntax.rs`) — `brush-parser` turns the line into an AST. brush understands the full bash grammar but only *parses*; it never evaluates.
2. **lower** (`lower.rs`) — the AST becomes a flat `Plan`: a sequence of and-or lists (`;`), each a first pipeline plus `&&`/`||` followers, each pipeline a list of stages, each stage raw argv words plus redirects. Lowering is structural and **honest**: any construct the executor does not handle becomes a clear `ShellError::Unsupported`, never a silent drop.
3. **execute** (`exec.rs`) — runs the plan against the [`NamespaceOps`](#the-namespaceops-seam) trait. Builtins run in-process; everything else launches as a child `#task`. Words are expanded here, at execution time, so a command sees same-line effects (`export X=1; echo $X`, `false; echo $?`).

## The NamespaceOps seam

The shell's *only* contact with the outside world is the `NamespaceOps` trait (`ns.rs`): stdout/stderr, `exists`, `read_file`/`write_file`, `#pipe` alloc/read/write, and `spawn`. The wasm guest (`fixtures/shell-src`) backs it with WASI over the task's namespace (`#task`, `#pipe`, files); host unit tests back it with an in-memory fake that simulates pipes and external commands, so whole pipelines run on the host with no wasm. That seam is why parsing, lowering, and the executor are all host-testable.

## What runs today

Simple commands and arguments; quote removal and `$VAR` / `${VAR}` / `$?` expansion; `;` sequences; `|` pipelines; `&&`/`||` short-circuit; `<` / `>` / `>>` file redirects; the `echo`, `cat`, `pwd`, `env`, `true`, `false`, `:`, `exit`, `cd`, `export`, `unset` builtins; the pipeable `tool PATH [PARAMS_JSON]` one-shot job-protocol client (`tool /n/upper < input.txt > output.txt` drives a mounted ToolFS through its visible files — `new`, `in`, `ctl run`, `out`, `result.json`, with `close` on success only — and on a failed job prints one `tool: <kind>: <message>` line to stderr, exits non-zero, and leaves the job retained behind its `job:` breadcrumb for post-mortem inspection; see `docs/toolfs.md`); and external command launch (resolved from `bin`, inheriting the shell's exported env). Builtins split into *pipeable* (compose in pipelines) and *special* (`cd`/`export`/`unset`, single-stage only — piping one is an honest error).

Everything else listed under honest limits above is parsed but reported as unsupported.

## Interactive mode

Run without `-c` and the shell is a REPL (`run_repl`): it renders the prompt (`$PS1`, default `\w $ `, with a `[code]` prefix when `$?` is non-zero), reads stdin byte-wise, and owns the cooked-line behavior per ADR 0003 — echo, backspace (`0x7f`/`0x08`), Ctrl-C cancels the line and reprompts, Ctrl-D on an empty line (or end-of-stream) exits, Enter dispatches the line through the same parse → lower → execute path as `-c`. The host side makes this real: `fd_read` in `wanix-wasi-host` now *blocks* on a not-ready device fd (bounded-backoff readiness parking, ADR 0010 tier 2), so a shell whose fd 0 is `#term/<id>/program` parks until the terminal client types. The end-to-end proof is `shell_repl_serves_a_terminal_session_over_blocking_reads` in `crates/wanix-wasm/src/driver.rs`.

Deliberately not built yet: tab completion, history, and cursor movement/arrow keys (escape sequences are ignored). The next steps are those, plus a CLI entry that binds a native terminal to a wasm shell task the way `qjs-shell` does for qjs.
