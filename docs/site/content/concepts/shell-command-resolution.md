---
title: Shell command resolution and the command package
slug: concepts/shell-command-resolution
pageType: concept
oneLiner: A bare command name resolves to a program in a bin directory (preferring <name>.wasm); installable commands are checked-in wasm fixtures assembled by command_bin() — so jq is just a command, not a shell builtin.
audience: [developer]
tags: [shell, commands, wasm, fixtures]
sourceRefs:
  - crates/wanix-sh/src/resolve.rs
  - crates/wanix-sh/src/ns.rs
  - crates/wanix-wasm/src/commands.rs
  - crates/wanix-wasm/fixtures/commands/HOW-TO-ADD-A-COMMAND.md
  - crates/wanix-wasm/fixtures/commands/jaq/src/main.rs
seeAlso:
  - concepts/wanix-sh
  - concepts/compiled-wasm-task-driver
  - devices/task
prerequisites:
  - concepts/wanix-sh
usedInFlows: []
honestLimits:
  - "The search path is the hardcoded relative list `bin`, `usr/bin` (no `$PATH` yet). Paths are Wanix-relative — `bin/jaq.wasm`, never `/bin/...`."
  - "Resolution prefers `<name>.wasm` because the wasm task driver only claims programs ending in `.wasm`; a bare hit is the fallback."
  - "Checked-in `.wasm` artifacts add repo size (jaq.wasm is ~1.5 MB); a growing command set may warrant git-lfs for fixtures/commands/*.wasm."
---

# Shell command resolution and the command package

A bare command name resolves to a program in a `bin` directory (preferring `<name>.wasm`); installable commands are checked-in wasm fixtures assembled by `command_bin()` — so `jq` is just a command, not a shell builtin.

The [shell](wanix-sh) decided early that `jq` would **not** be special-cased. Once the shell can resolve a name to a program and pipe it, a JSON filter is an ordinary external command. This page is how that resolution works and how to add more commands.

## Resolution

`resolve_command` (`resolve.rs`) is pure — it only reads `NamespaceOps::exists` — so a future tab-completion module can reuse it to enumerate candidates:

1. A builtin name is handled by the executor before resolution is reached.
2. A name containing `/` is taken literally (relative to the namespace root).
3. A bare name is searched across `bin`, then `usr/bin`: for each, try `<dir>/<name>.wasm`, then `<dir>/<name>`. The `.wasm` form is preferred because [`WasmTaskDriver::check`](compiled-wasm-task-driver) only claims `.wasm` programs.
4. No match returns the bare name, so the spawn step reports an honest "not found" (status 127).

Resolution is cwd-independent: the search dirs are relative to the real namespace root, and the shell's logical cwd never moves the guest's process cwd.

## The command package

Each installable command lives under `crates/wanix-wasm/fixtures/commands/<name>/` as its **own** cargo workspace (so the parent Wanix workspace ignores it), built to a checked-in `<name>.wasm`. `commands.rs` exposes `COMMANDS` (the `(name, bytes)` set, embedded via `include_bytes!`) and `command_bin()`, which assembles an in-memory `bin` filesystem. Bind that at `bin` in a task's namespace and children inherit it through namespace cloning — so the shell finds `bin/<name>.wasm`.

The first command is `jaq`, a real jq clone (`jaq-core`/`jaq-std`/`jaq-json`, stdlib wired so `map`/`add`/`select`/`keys` work). It reads one JSON document from stdin, applies the filter from `argv[1]`, and writes compact JSON.

## Adding a command

Per `fixtures/commands/HOW-TO-ADD-A-COMMAND.md`: create `fixtures/commands/foo/` as an isolated workspace, write a normal `main` (read `args`/`stdin`, write `stdout`), build it to `wasm32-wasip1`, drop `foo.wasm` next to the others, and add one `COMMANDS` line. The shell can then run — and pipe — `foo`.
