---
title: "Walkthrough 2 — Feed env, cwd, stdin, and argv into a Script"
slug: recipes/walkthrough-2-process-context
pageType: flow-step
oneLiner: "Pass --env/--cwd/--stdin and -- argv and watch scriptArgs, std.getenv, fd 0, and #task/self/id flow in as task state — no globalThis.Wanix bridge."
audience: [newcomer, developer]
tags: [cli, shipped, qjs-task]
sourceRefs:
  - rust-walkthrough.md:90-139
  - crates/wanix-cli/src/qjs_args/common/options.rs:47-98
  - crates/wanix-cli/src/qjs_args/command.rs:25-33
seeAlso:
  - concepts/qjs-task
  - concepts/tasks-own-process-identity
  - concepts/wanix-backed-wasi
  - concepts/guest-js-guardrails
prerequisites:
  - recipes/walkthrough-1-run-js
usedInFlows:
  - {flow: learn/js-outside-chrome, step: 2}
honestLimits:
  - "The qjs subcommand mounts your script at the well-known path main.js, so scriptArgs[0] is always main.js, not your host path."
  - "Guest JavaScript should use qjs:std, qjs:os, scriptArgs, and #task — the globalThis.Wanix bridge is retired."
---

# Walkthrough 2 — Feed env, cwd, stdin, and argv into a Script

Pass `--env`/`--cwd`/`--stdin` and `--` argv and watch `scriptArgs`, `std.getenv`, fd 0, and `#task/self/id` flow in as task state — no `globalThis.Wanix` bridge.

**What & why.** [Walkthrough 1](/recipes/walkthrough-1-run-js) proved a `.js` file runs as a Wanix task. A real program also needs context: arguments, environment, a working directory, and a byte stream on stdin. In Wanix those are not browser globals and not host-process inheritance — they are *task state* that Wanix supplies and the guest reads through ordinary WASI (`qjs:std`, `qjs:os`, fd 0) and service files (`#task/self/id`). This walkthrough feeds all four in from the command line and reads them back out from inside QuickJS.

## 1. Write a tiny context script

You already built the CLI in Walkthrough 1; reuse `$WANIX`. Write a script that reads each kind of context through a different door:

```sh
mkdir -p /tmp/wanix-walkthrough
cat > /tmp/wanix-walkthrough/context.js <<'JS'
import * as std from "qjs:std";
import * as os from "qjs:os";

const bytes = new Uint8Array(64);
const n = os.read(0, bytes.buffer, 0, bytes.length);
const stdin = Array.from(bytes.slice(0, n))
  .map((byte) => String.fromCharCode(byte))
  .join("");

std.out.puts("argv: " + scriptArgs.join("|") + "\n");
std.out.puts("mode: " + std.getenv("MODE") + "\n");
std.out.puts("cwd source: " + std.loadFile("main.js").includes("qjs:std") + "\n");
std.out.puts("task: " + std.loadFile("#task/self/id").trim() + "\n");
std.out.puts("stdin: " + stdin + "\n");
std.out.flush();
JS
```

No `Wanix.*` calls — `scriptArgs` is a QuickJS global, `std.getenv` is the env, `os.read(0, …)` is a raw read on fd 0, and `#task/self/id` is a file path.

## 2. Run it with cwd, env, stdin, and argv

```sh
$WANIX qjs \
  --env MODE=walkthrough \
  --cwd app \
  --stdin "hello fd0" \
  /tmp/wanix-walkthrough/context.js \
  -- alpha "two words"
```

Each flag maps to one piece of task state. `--env KEY=VALUE`, `--cwd <wanix path>`, and `--stdin <text>` are all common `qjs` options (`crates/wanix-cli/src/qjs_args/common/options.rs:47-98`); everything after the bare `--` separator is forwarded verbatim as guest argv (`crates/wanix-cli/src/qjs_args/command.rs:25-33`).

## 3. Read the output

```text
argv: main.js|alpha|two words
mode: walkthrough
cwd source: true
task: 1
stdin: hello fd0
```

- **`argv: main.js|alpha|two words`** — your two post-`--` arguments arrived intact, including the quoted `two words`. The leading `main.js` is the script itself: the `qjs` subcommand mounts your file at the well-known path `main.js`, so `scriptArgs[0]` is always `main.js`, not the host path you typed (`crates/wanix-cli/src/qjs_args/command.rs:25`).
- **`mode: walkthrough`** — `std.getenv("MODE")` read the value you set with `--env`. The env is task state Wanix hands to the guest, not the shell's environment.
- **`cwd source: true`** — `std.loadFile("main.js")` resolved your script through the task's namespace and found the `qjs:std` import string. The `--cwd app` set the task's working directory without breaking that well-known mount.
- **`task: 1`** — `#task/self/id` is the task's own file in the `#task` service device, read like any other file.
- **`stdin: hello fd0`** — `os.read(0, …)` pulled the bytes you piped in with `--stdin` straight off file descriptor 0.

## 4. What happened

Every line above is one fact: *Wanix owns process identity and context; QuickJS is a tenant that reads it through standard interfaces.* argv, env, and fd 0 flow in through **live Wanix-backed WASI** rather than the host OS, and identity flows in through a **service file**. There is no `globalThis.Wanix` bridge — and there should not be one: guest JavaScript uses `qjs:std`, `qjs:os`, `scriptArgs`, and `#task` (`rust-walkthrough.md:136-139`). The same script would read the same context whether it ran here, in the browser cockpit, or against a namespace imported across the mesh, because the contract is files and fds, not an embedding.

## Next

Mount a real host directory into the task's namespace and read a file the engine never had baked in — the explicit-authority counterpart to this implicit context.

## See also

- [The qjs task](/concepts/qjs-task) — how a `.js` file becomes a Wanix task and where env/cwd/argv are applied.
- [Tasks own process identity](/concepts/tasks-own-process-identity) — why `#task/self/id` is a file and the engine is a tenant.
- [Wanix-backed WASI](/concepts/wanix-backed-wasi) — the WASI Preview 1 layer that carries fd 0, env, and namespace reads.
- [Guest JS guardrails](/concepts/guest-js-guardrails) — use `qjs:std`/`qjs:os`/`scriptArgs`/`#task`, not the retired `globalThis.Wanix` bridge.

## Status / honest limits

- The `qjs` subcommand always mounts your script at `main.js`, so `scriptArgs[0]` is `main.js`, not the host path you passed on the command line (`crates/wanix-cli/src/qjs_args/command.rs:25`).
- `--env`, `--cwd`, `--stdin`, and the `--` argv separator are the shipped common `qjs` options (`crates/wanix-cli/src/qjs_args/common/options.rs:47-98`); they set task state, not the host shell environment.
- Guest code should not reach for a `globalThis.Wanix` bridge — it is retired. Use `qjs:std`, `qjs:os`, `scriptArgs`, and `#task` service files.
