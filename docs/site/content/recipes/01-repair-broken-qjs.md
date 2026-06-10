---
title: Recipe 01 — Repair a Broken qjs Program with an Agent
slug: recipes/01-repair-broken-qjs
pageType: use-case
oneLiner: Hand a crashing qjs program to a Wanix-backed agent from inside the cockpit and watch it propose a patch, ask for approval, and write the fix back — all as plain reads and writes on the #agent service.
audience: [developer, newcomer]
tags: [recipe, agent, cockpit, services, cli, local-trust-only, caveat]
sourceRefs:
  - docs/recipes/01-repair-broken-qjs.md
  - crates/wanix-agent/src/files.rs:23-185
  - crates/wanix-agent/src/fake.rs:78-100
  - crates/wanix-cli/src/serve/roots.rs:238-243
  - workbench/src/web/agent-repair-demo.ts:67-260
seeAlso:
  - devices/agent
  - concepts/approvals-as-files
  - concepts/fakeengine-vs-codex
  - concepts/browser-cockpit
  - recipes/05-two-agents-collaborate
prerequisites:
  - devices/agent
  - concepts/approvals-as-files
usedInFlows:
  - {flow: agent-on-your-files, step: 2}
honestLimits:
  - The served #agent is a deterministic FakeEngine, not a live LLM; it parks an approval on any `approve:`-prefixed prompt. The real codex engine is the local-trust `wanix agent` CLI path only.
  - The #agent device, like #task and #cpu, is an exec device and is local-trust only — it is not exposed to untrusted or public peers, and there are no hard CPU/memory limits yet.
  - serve handles one 9P frame at a time per connection, so the cockpit reads the streaming `events` file in non-blocking chunks rather than holding a blocking read open.
---

# Recipe 01 — Repair a Broken qjs Program with an Agent

Hand a crashing qjs program to a Wanix-backed agent from inside the cockpit and watch it propose a patch, ask for approval, and write the fix back — all as plain reads and writes on the `#agent` service.

**What & why.** You have a script that crashes. The usual fix loop is a chat window in another tab: copy the error out, paste a diff back, apply it by hand. Wanix collapses that loop into the filesystem. The agent is a device — `#agent` — and "ask the agent to fix this" is a sequence of `read` and `write` calls on files under `#agent/<id>/`. The powerful step (editing your file) parks as an approval request you can see and gate. Nothing magic, nothing out-of-band: the entire LLM contract is reads and writes on a tree, and you can drive it from the cockpit or by hand from any 9P client.

## 1. Set up a tiny project with one broken file

First build the binary and alias it, then drop a script that crashes on a missing identifier:

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix='./target/debug/wanix'

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

Confirm it is broken before handing it over. (`--cwd` is a *namespace* path, so bind the host dir in with `--mount` first — see [build & install](/reference/build-and-install) on the two kinds of paths.)

```sh
wanix qjs --mount /tmp/repair-demo/agent=agent --cwd agent \
  /tmp/repair-demo/agent/broken.js
# ReferenceError: missingValue is not defined  (non-zero exit; out/result.txt never written)
```

## 2. Start serve with the cockpit bundle and services

The cockpit needs the workbench HTML bundle and `--wanix-services`, which binds the service devices into the served namespace. Add a loopback raw-9P door (`--p9`) too — the by-hand CLI loop in section 4 dials it:

```sh
wanix serve \
  --root /tmp/repair-demo \
  --listen 127.0.0.1:7654 \
  --p9 127.0.0.1:7664 \
  --bundle workbench-fs9p \
  --wanix-services
```

With `--wanix-services`, `roots.rs:238-243` binds `#agent` backed by a deterministic `FakeEngine` — `AgentDevice::new(Arc::new(FakeEngine))`. The served agent deliberately uses the fake engine: the real `codex` bridge needs auth and unattended execution, so it stays local-trust on the CLI. The wire shape the fake produces is identical to a real engine's, so everything you learn here transfers.

## 3. Open the cockpit and trigger the repair

Open `http://127.0.0.1:7654/?bundle=workbench-fs9p`. The Wanix sidebar exposes a System Actions panel; run **Run Agent Repair Demo**. That calls `runAgentRepair` in `agent-repair-demo.ts:165`, which opens a side-by-side diff of `broken.js` before and after the edit when the turn completes.

You do not need the browser. The cockpit is a thin client — `runAgentRepair` is about seventy lines and does nothing the CLI cannot. The next section is the same loop by hand.

## 4. Under the hood: new → prompt → events → pending → ctl → close

Every step is a file operation on `#agent`.

1. **Allocate a session** by reading `#agent/new`. `NewAgentFile::read` (`files.rs:40`) allocates on first read and returns an id like `1\n`.
   ```sh
   id=$(wanix mount-cat tcp://127.0.0.1:7664 '#agent/new' | tr -d '\n')
   ```
2. **Submit the prompt** by writing `#agent/<id>/prompt`. `PromptFile::write` (`files.rs:101`) forwards each write as a turn. The cockpit writes an `approve:`-prefixed plan; the fake engine treats that prefix as the request-for-approval contract (`fake.rs:85-96`), parking a pending request `req-<turn>` instead of editing immediately.
   ```sh
   wanix mount-write tcp://127.0.0.1:7664 "#agent/$id/prompt" \
     "approve: declare missingValue so broken.js writes out/result.txt"
   ```
3. **Read the parked request.** `EventsFile::read` (`files.rs:125`) streams JSONL — `turn.started`, then `approval.needed` with an `id` — but it is a *stream*: a collected `mount-cat` of `events` blocks until the session closes (the cockpit polls it in chunks instead). The non-blocking way to see the parked request is the `pending` snapshot:
   ```sh
   wanix mount-cat tcp://127.0.0.1:7664 "#agent/$id/pending"
   # -> [{"action":"declare missingValue ...","id":"req-1"}]
   ```
4. **Approve via `ctl`.** `CtlFile::write` (`files.rs:160-180`) accepts `approve <id>`, `deny <id>`, and `close`; an unknown verb is `NotSupported`. Approving routes through `AgentDevice::resolve` and unblocks the turn, which emits a final `message` and `turn.completed`.
   ```sh
   wanix mount-write tcp://127.0.0.1:7664 "#agent/$id/ctl" "approve req-1"
   ```
5. **Close the session** so the device drops it and EOFs open readers:
   ```sh
   wanix mount-write tcp://127.0.0.1:7664 "#agent/$id/ctl" "close"
   ```

The cockpit's `streamUntilCompletion` loop (`agent-repair-demo.ts:203`) does exactly this, polling `events` in chunks and popping a modal at `approval.needed` so a human, not the engine, decides.

## 5. Inspect the result

A successful repair leaves a normal program:

```sh
wanix qjs --mount /tmp/repair-demo/agent=agent --cwd agent \
  /tmp/repair-demo/agent/broken.js
# wrote out/result.txt
cat /tmp/repair-demo/agent/out/result.txt
```

The cockpit also refreshes the file tree and shows the before/after diff so you can read exactly what was inserted.

## What you exercised

- **The `#agent` device shape:** `new`, `<id>/prompt`, `<id>/events`, `<id>/pending`, `<id>/ctl`, `<id>/status`, `<id>/reply`. The full LLM session reduces to read/write on this tree — see [the agent device](/devices/agent).
- **The trust-boundary handoff:** powerful actions park in `pending` and do nothing until a human writes `approve <id>` to `ctl`. That gate is [approvals as files](/concepts/approvals-as-files) — the same shape works for [two agents collaborating](/recipes/05-two-agents-collaborate).

To run the same loop against a real LLM, switch to the CLI: `wanix agent --cwd /tmp/repair-demo/agent "Repair broken.js so it writes out/result.txt"` defaults to the codex engine. The served cockpit stays on the fake engine; the file shape is identical.

## See also

- [The #agent device](/devices/agent)
- [Approvals as files](/concepts/approvals-as-files)
- [FakeEngine vs codex](/concepts/fakeengine-vs-codex)
- [The browser cockpit](/concepts/browser-cockpit)
- [Recipe 05 — Two agents collaborate](/recipes/05-two-agents-collaborate)
- Flow: [Agent on your files](/learn/agent-on-your-files)

## Status / honest limits

- **The served `#agent` is a deterministic `FakeEngine`, not a live LLM.** It is wired in `roots.rs:238-243` and parks an approval on any `approve:`-prefixed prompt (`fake.rs:85-96`); it does not call a model. The real codex engine is the local-trust `wanix agent` CLI path only, because it needs auth and unattended execution.
- **`#agent` is an exec device and is local-trust only.** Like `#task` and `#cpu`, it is not exposed to untrusted or public peers, and there are no hard CPU or memory limits yet. The approval gate is the trust boundary inside a trusted node, not a sandbox for arbitrary hostile code.
- **serve handles one 9P frame at a time per connection.** The cockpit therefore reads the streaming `events` file in non-blocking chunks and gates on the `pending` snapshot rather than holding a blocking read open — a blocking read would not interleave with the writes on the same connection.
