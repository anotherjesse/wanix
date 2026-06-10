# Recipe 01 — Repair my broken qjs program with an agent

Goal: take a tiny qjs program that crashes with a `ReferenceError`, hand it to a
Wanix-backed agent from inside the cockpit, and watch it propose a patch, ask
for approval, and write the fix back — all as plain reads and writes on the
`#agent` service.

This is the demo path the workbench calls "Run Agent Repair Demo". Under the
hood it is just a few file operations against the `#agent/<id>/...` service
files exposed by `wanix serve --wanix-services`.

Source:

- Agent device: [`crates/wanix-agent/src/lib.rs`][agent-lib],
  [`crates/wanix-agent/src/files.rs`][agent-files],
  [`crates/wanix-agent/src/engine.rs`][agent-engine].
- Service binding: [`crates/wanix-cli/src/serve/roots.rs`][serve-roots].
- Workbench driver: [`workbench/src/web/agent-repair-demo.ts`][workbench-demo].

[agent-lib]: ../../crates/wanix-agent/src/lib.rs
[agent-files]: ../../crates/wanix-agent/src/files.rs
[agent-engine]: ../../crates/wanix-agent/src/engine.rs
[serve-roots]: ../../crates/wanix-cli/src/serve/roots.rs
[workbench-demo]: ../../workbench/src/web/agent-repair-demo.ts

## 1. Set up a tiny project with one broken file

Make a workspace and drop in a qjs script that crashes on a missing
identifier. We will let the agent repair this exact file.

```sh
mkdir -p /tmp/repair-demo/agent
cat > /tmp/repair-demo/agent/broken.js <<'JS'
import * as std from "qjs:std";
import * as os from "qjs:os";

os.mkdir("out", 0o777);
if (typeof missingValue === "undefined") {
  std.err.puts("ReferenceError: missingValue is not defined\n");
  std.exit(1);
}
std.writeFile("out/result.txt", missingValue.toUpperCase() + "\n");
std.out.puts("wrote out/result.txt\n");
JS
```

Confirm the program is in fact broken before we hand it to the agent:

```sh
wanix qjs --cwd /tmp/repair-demo/agent broken.js
```

You should see `ReferenceError: missingValue is not defined` on stderr and a
non-zero exit. The directory `/tmp/repair-demo/agent/out` exists but
`result.txt` was never written.

## 2. Start `wanix serve` with the cockpit bundle and services

The cockpit needs two things: the workbench-fs9p HTML bundle (the browser
editor entry point) and `--wanix-services`, which binds the `#agent`,
`#task`, `#term`, `#pipe`, `#kv`, `#plumb`, and `#cas` service devices into
the served namespace.

```sh
wanix serve \
  --root /tmp/repair-demo \
  --addr 127.0.0.1:7654 \
  --bundle workbench-fs9p \
  --wanix-services
```

The exact flag list is defined in
[`crates/wanix-cli/src/serve/command.rs`][serve-command]: `--root DIR` (or a
positional directory), `--addr HOST:PORT` (or `--listen HOST:PORT`),
`--bundle NAME`, `--wanix-services`, and `--once`. With `--wanix-services`,
`serve/roots.rs` calls `bind_host_and_terminal`, which binds `#agent` backed
by a deterministic `FakeEngine` (see lines 156–164 of [`roots.rs`][serve-roots]):

```rust
namespace.bind(
    Arc::new(AgentDevice::new(Arc::new(FakeEngine))),
    ".",
    "#agent",
    BindOptions::default(),
)?;
```

The served `#agent` deliberately uses the fake engine because the real
`codex app-server` bridge is local-trust only. The recipe here therefore works
end to end against the served cockpit with no network keys; the wire shape is
identical to what a real engine produces.

On startup `serve` prints:

```
wanix serve: serving /tmp/repair-demo files with Wanix overlay
wanix serve: bundle available at http://127.0.0.1:7654/?bundle=workbench-fs9p
```

[serve-command]: ../../crates/wanix-cli/src/serve/command.rs

## 3. Open the cockpit and trigger the repair

Open the bundle URL in a browser:

```
http://127.0.0.1:7654/?bundle=workbench-fs9p
```

The Wanix sidebar exposes a `System Actions` panel. From the command palette
or that panel, run **Run Agent Repair Demo**. The command is registered in
[`workbench/src/web/extension.ts`][workbench-ext] (search for
`workbench.runAgentRepairDemo`), which in turn calls `runAgentRepair` in
[`agent-repair-demo.ts`][workbench-demo].

If you prefer the more general case — the file you are already editing —
choose **Fix Current Wanix Program** instead. It runs the same loop against
the active editor's qjs file.

[workbench-ext]: ../../workbench/src/web/extension.ts

## 4. What happens under the hood

`runAgentRepair` performs exactly the steps below on the `#agent` service.
You can do them by hand from any 9P client; the cockpit just automates them.

1. **Allocate a session by reading `#agent/new`.** The device's
   [`NewAgentFile`][agent-files] reads back an id on first read:

   ```sh
   # Equivalent to fsys.readFile("/#agent/new") in the workbench.
   id=$(wanix mount-cat tcp://127.0.0.1:7654 '#agent/new' | tr -d '\n')
   echo "session: $id"
   ```

   `AgentDevice::alloc` (in [`lib.rs`][agent-lib]) starts an engine session
   and inserts it under `#agent/<id>/...`. Reading `new` then yields, e.g.,
   `1\n`.

2. **Submit the repair prompt by writing `#agent/<id>/prompt`.** The
   [`PromptFile`][agent-files] forwards each write to
   `AgentSession::submit`. The workbench's `REPAIR_PROMPT` (see
   [`agent-repair-demo.ts`][workbench-demo] lines 27–31) ends with:

   > "When you would apply a change, pause for approval via #agent ctl."

   That suffix is what makes the engine park for human approval before
   editing your file:

   ```sh
   wanix mount-write tcp://127.0.0.1:7654 \
     "#agent/$id/prompt" \
     "approve: edit /tmp/repair-demo/agent/broken.js to declare missingValue"
   ```

   The leading `approve:` is the deterministic fake's contract for asking
   for approval (see [`fake.rs`][agent-fake] lines 84–96); a real codex
   engine wraps this same shape around its own tool-call previews.

3. **Watch `#agent/<id>/events`.** [`EventsFile`][agent-files] blocks on
   `AgentSession::read_events`, returning one JSONL line per event:

   ```sh
   wanix mount-cat tcp://127.0.0.1:7654 "#agent/$id/events"
   ```

   You will see a `turn.started`, then an `approval.needed` with an `id`
   field. The cockpit pops a modal at this point. With the bare CLI, you
   inspect what is pending:

   ```sh
   wanix mount-cat tcp://127.0.0.1:7654 "#agent/$id/pending"
   # -> [{"id":"req-1","action":"edit /tmp/repair-demo/agent/broken.js ..."}]
   ```

4. **Approve via `#agent/<id>/ctl`.** The [`CtlFile`][agent-files] write
   handler accepts `approve <request-id>`, `deny <request-id>`, and `close`:

   ```sh
   wanix mount-write tcp://127.0.0.1:7654 \
     "#agent/$id/ctl" "approve req-1"
   ```

   Internally that calls `AgentDevice::resolve` → `AgentSession::resolve`,
   which unblocks the turn. The events stream emits a final `message` and a
   `turn.completed`. The cockpit's `streamUntilCompletion` loop in
   [`agent-repair-demo.ts`][workbench-demo] watches for `"turn.completed"`
   and stops reading.

5. **Close the session.** The workbench writes `close` to `ctl` after the
   turn so the device drops the session and frees the engine handle:

   ```sh
   wanix mount-write tcp://127.0.0.1:7654 "#agent/$id/ctl" "close"
   ```

[agent-fake]: ../../crates/wanix-agent/src/fake.rs

## 5. Inspect the result

The cockpit refreshes the Wanix file tree and opens a side-by-side diff of
`broken.js` (before / after the agent's edit). You can also look at the
artifacts directly:

```sh
cat /tmp/repair-demo/agent/broken.js
cat /tmp/repair-demo/agent/out/result.txt
ls /tmp/repair-demo/agent/out/
```

If the run produced a repair report (the `Run Agent Repair Demo` path always
does), it lives at `/agent/out/broken.repair-report.md` and lists every
operation: prompt, transcript, before/after snapshot, approval decision,
rerun exit code, and the final result file.

A successful repair leaves a normal qjs program that runs cleanly:

```sh
wanix qjs --cwd /tmp/repair-demo/agent broken.js
# wrote out/result.txt
cat /tmp/repair-demo/agent/out/result.txt
# <the value the agent chose, upper-cased>
```

## What you just exercised

- The `#agent` device shape: `new`, `<id>/prompt`, `<id>/events`,
  `<id>/pending`, `<id>/ctl`, `<id>/status`, `<id>/reply`. Every one is a
  plain file — the entire LLM contract reduces to read/write on this tree.
- The trust-boundary handoff: powerful actions (edits, command runs) park as
  approval requests visible in `pending`; nothing happens until a human
  writes `approve <id>` to `ctl`. That gate lives in
  [`files.rs`][agent-files] (`CtlFile::write`) and
  [`lib.rs`][agent-lib] (`AgentDevice::resolve`).
- The cockpit as a thin client: `runAgentRepair` in
  [`agent-repair-demo.ts`][workbench-demo] is ~70 lines and does nothing the
  CLI cannot do. The `#agent` service is the contract; the cockpit is one
  client.

To run the same loop against a real LLM, swap the engine on the CLI path:

```sh
wanix agent --cwd /tmp/repair-demo/agent \
  "Repair broken.js so it writes out/result.txt"
```

The CLI in [`crates/wanix-cli/src/agent.rs`][cli-agent] defaults to
`CodexEngine` (the `codex app-server` bridge) and falls back to `FakeEngine`
under `--fake`. The served cockpit stays on the fake engine for safety; the
service-file shape stays identical, so anything you learned here transfers.

[cli-agent]: ../../crates/wanix-cli/src/agent.rs
