---
title: FakeEngine vs codex (Exec Local-Trust-Only)
slug: concepts/fakeengine-vs-codex
pageType: concept
oneLiner: The served #agent uses the deterministic FakeEngine; the real codex app-server engine is local-trust only on the CLI path; the file shape is identical either way.
audience: [developer, visionary]
tags: [agent, mesh, local-trust-only, caveat, cli, shipped]
sourceRefs:
  - crates/wanix-cli/src/serve/roots.rs:163-170
  - crates/wanix-cli/src/agent.rs:42-91
  - crates/wanix-agent/src/codex.rs:48-102
  - crates/wanix-agent/src/fake.rs:1-175
  - workbench/src/web/agent-repair-demo.ts:36-66
seeAlso:
  - devices/agent
  - concepts/approvals-as-files
  - concepts/trust-boundary-gaps
  - concepts/agents-as-operators
prerequisites:
  - devices/agent
usedInFlows:
  - {flow: agent-on-your-files, step: 4}
honestLimits:
  - The served #agent is a deterministic FakeEngine, not a live LLM; do not claim the cockpit runs a real model.
  - Real codex runs only on the local-trust `wanix agent` CLI path; it requires codex auth and an installed `codex` binary.
  - The cockpit agent-repair demo computes its patch in-process and uses #agent only for session identity + the approval gate; it falls back to the in-process patch if the device is unreachable.
canonicalCaveatFor: [served-agent-is-fakeengine]
---

# FakeEngine vs codex (Exec Local-Trust-Only)

The served #agent uses the deterministic FakeEngine; the real codex app-server engine is local-trust only on the CLI path; the file shape is identical either way.

Two engines back the same `#agent` device. One is a fixed-reply stub with no network and no subprocess; the other shells out to a real `codex app-server` against a confined world. Which one you get depends entirely on *where* the device is bound — served over the network, or local on the CLI. The file contract you read and write does not change between them, so everything you learn driving the stub transfers verbatim to the real engine. This page is about that swap, and about why the served side is *not* a live LLM.

## Show: same files, different reply

Start a served namespace and drive the agent over 9P:

```sh
cargo build --package wanix-cli; alias wanix-rust='./target/debug/wanix-rust'
wanix-rust serve --wanix-services &

# allocate a session, prompt it, read the reply
id=$(cat '#agent/new')
echo 'fix the failing test' > "#agent/$id/prompt"
cat "#agent/$id/events"
# ... {"t":"message","text":"you said: fix the failing test"} ...
```

The reply is literally `you said: <your prompt>`. That is the FakeEngine talking. Now the same files on the CLI path:

```sh
wanix-rust agent --cwd ./myproject 'fix the failing test'
# streams real codex turn events: tool calls, message deltas, turn.completed
```

Same `new`/`prompt`/`events`/`status` files (see [the #agent device](/devices/agent)). Same event vocabulary — `turn.started`, `message.delta`, `message`, `tokens`, `turn.completed`. Only the *content* of the messages differs, because a different `AgentEngine` is bound underneath.

## Name it: the engine is a bind, the wire is fixed

`AgentDevice::new(engine)` takes any `Arc<dyn AgentEngine>`. The device owns the file shape — session ids, the event stream, the approval gate — and delegates only the *thinking* to the engine. So the choice of engine is one constructor argument, and it is made in exactly two places.

**Served path binds FakeEngine.** When `serve --wanix-services` composes the namespace, it binds the agent device with the stub (`crates/wanix-cli/src/serve/roots.rs:163-170`):

```rust
// Served #agent uses the deterministic fake engine: the real codex bridge is
// local-trust only (auth + unattended execution) and stays on the CLI path.
namespace.bind(
    Arc::new(AgentDevice::new(Arc::new(FakeEngine))),
    ".", "#agent", BindOptions::default(),
)?;
```

The comment states the reason flatly. Real codex needs working auth (`CODEX_HOME`/`~/.codex`) and runs commands unattended; exposing that over a served, network-reachable endpoint would put an LLM-driven exec plane behind the wire. Exec devices are local-trust only, so the served side gets the stub instead. See [exec devices are local-trust only](/concepts/trust-boundary-gaps).

**CLI path defaults to codex.** `wanix agent <prompt>` builds a `CodexEngine` unless you pass `--fake` (`crates/wanix-cli/src/agent.rs:82-91`): `--fake` selects `FakeEngine`, `--world DIR` selects `CodexEngine::with_wanix_world` (the agent's *entire* filesystem becomes that host directory), and the bare default is `CodexEngine::new(codex, None, cwd)`. The codex engine spawns `codex app-server --listen stdio://`, does the JSON-RPC handshake, and normalizes its notification stream into `#agent` events (`crates/wanix-agent/src/codex.rs:48-102`). It is a real subprocess against a real model, gated behind your explicit local trust.

## What the FakeEngine actually does

The stub is small and deterministic on purpose (`crates/wanix-agent/src/fake.rs:1-175`). On `submit(prompt)`:

- An ordinary prompt streams `turn.started`, two `message.delta` chunks, a final `message` of `you said: <prompt>`, a `tokens` line, then `turn.completed`. No network, no model, no randomness.
- A prompt prefixed `approve:` instead parks a pending approval and emits `approval.needed`; the turn *blocks* until you resolve it through `ctl`. `resolve(id, "approve")` emits `approved: <action>` and completes the turn; anything else declines it.

That second branch exists so the device's trust-boundary plumbing — `pending`, `ctl`, `reply` — is exercisable without a live LLM. The approval gate is the same code whether the engine is fake or codex; only the stub lets you test it offline. See [approvals as files](/concepts/approvals-as-files).

## Why identical file shape is the whole point

Because `AgentDevice` owns the contract and engines only fill in replies, the FakeEngine is a *fixture for the real thing*, not a different system. A cockpit panel, a shell script, or a sub-agent that drives `#agent/<id>/prompt` and reads `#agent/<id>/reply` does not know or care which engine answered. Swap `FakeEngine` for `CodexEngine` and the caller is unchanged. This is the same lesson as everywhere in Wanix: the capability is the file shape, the implementation is a bind behind it. An agent driving another agent over the mesh (`#agent/<id>/reply`) composes the same way — see [agents as operators](/concepts/agents-as-operators).

## The cockpit agent-repair demo, honestly

The "Run Agent Repair Demo" action in the [browser cockpit](/use-cases/browser-cockpit) looks like a live agent fixing your code. It is not. The cockpit *computes* the concrete patch in-process — a deterministic transform that inserts a missing declaration — and uses `#agent` only for session identity and the approval gate (`workbench/src/web/agent-repair-demo.ts:36-66`). It submits the patch plan as an `approve:`-prefixed prompt so the served FakeEngine parks an approval, resolves it through `ctl`, then applies the precomputed patch. If services are disabled and the device is unreachable, it falls back to the in-process patch directly so the demo never hard-fails. The demo shows the *device's* approval flow truthfully; it does not show a model reasoning about your code.

## See also

- [The #agent device](/devices/agent) — the `new`/`prompt`/`events`/`pending`/`ctl`/`reply`/`status` file contract both engines share.
- [Approvals as files](/concepts/approvals-as-files) — the `approve:` gate the FakeEngine exists to exercise.
- [Exec devices are local-trust only](/concepts/trust-boundary-gaps) — why the served side cannot be the real codex.
- [Agents as operators](/concepts/agents-as-operators) — agents delegating to agents over the same files.

## Status / honest limits

- **The served #agent is a deterministic FakeEngine, not a live LLM.** It replies `you said: <prompt>` with fixed token counts and no network. Do not claim the served cockpit runs a real model (`crates/wanix-cli/src/serve/roots.rs:163-170`, `crates/wanix-agent/src/fake.rs:57-70`).
- **Real codex is local-trust CLI only.** `wanix agent` defaults to `CodexEngine`, which requires an installed `codex` binary and working codex auth (`CODEX_HOME`/`~/.codex`); without them the engine cannot start (`crates/wanix-agent/src/codex.rs:48-54`). Use `--fake` to drive the stub from the CLI.
- **The agent-repair demo precomputes its patch.** It uses `#agent` for session identity and the approval gate, not for reasoning, and falls back to the in-process patch if the device is unreachable (`workbench/src/web/agent-repair-demo.ts:36-66`).
