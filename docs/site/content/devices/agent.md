---
title: "#agent — an LLM Session as Files"
slug: devices/agent
pageType: device
oneLiner: "new/prompt/events/reply/pending/ctl/status — the entire LLM contract reduces to read/write on a small file tree, with approvals as files."
audience: [newcomer, developer, visionary]
tags: [mesh, service-device, shipped, local-trust-only, caveat]
sourceRefs:
  - crates/wanix-agent/src/lib.rs:56-231
  - crates/wanix-agent/src/files.rs:23-185
  - crates/wanix-agent/src/engine.rs:27-166
  - crates/wanix-agent/src/path.rs:3-46
  - crates/wanix-agent/src/fake.rs:19-175
  - crates/wanix-agent/src/router.rs:27-83
  - crates/wanix-agent/src/remote.rs:33-168
  - crates/wanix-cli/src/serve/roots.rs:164-166
  - crates/wanix-cli/src/agent.rs:82-91
  - crates/wanix-cli/src/serve/http/agent.rs:1-44
seeAlso:
  - concepts/approvals-as-files
  - concepts/fakeengine-vs-codex
  - concepts/agents-as-operators
  - concepts/send-agent-to-the-data
  - concepts/blocking-stream-eof-contract
  - recipes/01-repair-broken-qjs
  - recipes/05-two-agents-collaborate
prerequisites:
  - concepts/service-devices
  - concepts/approvals-as-files
usedInFlows:
  - {flow: agent-on-your-files, step: 2}
honestLimits:
  - "The served #agent is a deterministic FakeEngine, not a live LLM; real codex is the local-trust `wanix agent` CLI path only."
  - "Exec devices including #agent are local-trust only; not exposed to untrusted or public peers, and there are no hard CPU/memory limits yet."
  - "ctl supports exactly three verbs: close, approve <req>, deny <req>."
canonicalCaveatFor: []
---

# #agent — an LLM Session as Files

new/prompt/events/reply/pending/ctl/status — the entire LLM contract reduces to read/write on a small file tree, with approvals as files.

An agent is usually a process you talk to over a socket with a bespoke JSON-RPC protocol. `#agent` makes it a directory. You `cat new` to get a session, `echo` a prompt into `prompt`, `cat events` to watch the reply stream, and `echo approve req-1 > ctl` when the agent wants to do something you have to sign off on. Nothing about that surface knows it is an LLM — it is the same read/write contract every other Wanix device speaks, which is exactly why an agent imports across the mesh, gets re-rooted by a capability bind, and gets driven by a browser cockpit without any agent-specific networking. This page is the file-by-file reference for that surface, the two engines behind it, and the honest boundary of what the served device actually is.

## Allocate a session: `#agent/new`

`new` is read-only. The first `read` on the file allocates a session, starts an engine session, and returns the new id followed by a newline (`crates/wanix-agent/src/files.rs:40-53`, `crates/wanix-agent/src/lib.rs:87-94`). Opening it write-anything fails: every snapshot file in the device goes through `require_read_only`, which rejects `write`, `create`, or `truncate` (`crates/wanix-agent/src/files.rs:8-13`).

```sh
cargo build --package wanix-cli
alias wanix='./target/debug/wanix'

# served device: read `new`, get an id back
cat '#agent/new'      # -> 1
```

The id is a monotonic counter (`next_id`, saturating-incremented under the device lock) stringified, so the first session is `1`, the next `2`, and so on (`crates/wanix-agent/src/lib.rs:87-94`). Listing the device root shows `new` plus one directory per live session (`crates/wanix-agent/src/lib.rs:200-215`).

## Drive the turn: the session files

Each session is a directory `#agent/<id>/` exposing seven files (`crates/wanix-agent/src/lib.rs:216-227`, parsed in `crates/wanix-agent/src/path.rs:35-46`):

- **`id`** — read returns the session id (`<id>\n`); a fixed snapshot (`crates/wanix-agent/src/lib.rs:140-144`).
- **`prompt`** — write-to-submit. Each write trims the trailing newline and submits the text as a new turn (`crates/wanix-agent/src/files.rs:96-110`). Mode `0o666`.
- **`events`** — read the normalized JSONL event stream. The read **blocks** until the next event arrives or the session closes, at which point it returns `Ok(0)` — end-of-stream (`crates/wanix-agent/src/files.rs:124-136`, `crates/wanix-agent/src/engine.rs:139-166`).
- **`reply`** — read blocks until the latest turn completes, then returns the single final assistant message and EOF (`crates/wanix-agent/src/lib.rs:161-165`, `crates/wanix-agent/src/engine.rs:78-80`). This is the one-shot counterpart to the streaming `events` — see [live stream vs. one-shot](/concepts/live-stream-vs-one-shot) and [the blocking-stream EOF contract](/concepts/blocking-stream-eof-contract).
- **`status`** — read a one-line snapshot: engine label, state, turn count (`crates/wanix-agent/src/lib.rs:145-149`, `crates/wanix-agent/src/fake.rs:111-124`).
- **`pending`** — read a JSON array of open approval requests, one object per request (`crates/wanix-agent/src/lib.rs:154-160`).
- **`ctl`** — write control verbs (below).

The full loop, all of it `echo` and `cat`:

```sh
echo 'fix the failing test' > '#agent/1/prompt'
cat '#agent/1/events'      # streams turn.started / message.delta / message / turn.completed
cat '#agent/1/reply'       # blocks, then the final message, then EOF
cat '#agent/1/status'      # -> fake idle turns=1
```

## Approvals are files: `pending` + `ctl`

A powerful action — running a command, editing a file — does not just happen. It **parks** as an approval request in `pending`, and the turn does not complete until a human resolves it. In the deterministic engine, a prompt prefixed `approve:` exercises this path: it emits an `approval.needed` event, writes a request into `pending`, and waits (`crates/wanix-agent/src/fake.rs:80-101`). Nothing runs until you write a decision to `ctl`:

```sh
echo 'approve: rm -rf /tmp/build' > '#agent/1/prompt'
cat  '#agent/1/pending'           # -> [{"id":"req-1","action":"rm -rf /tmp/build"}]
echo 'approve req-1' > '#agent/1/ctl'   # the turn now completes
```

`ctl` accepts exactly three verbs, parsed whitespace-split (`crates/wanix-agent/src/files.rs:160-180`):

- `close` — closes and removes the session; the event stream ends (`crates/wanix-agent/src/lib.rs:102-109`).
- `approve <req>` — resolves request `<req>` as approved.
- `deny <req>` — resolves it as declined.

`approve` and `deny` require a request id; anything else returns `NotSupported`. Because the gate is a file write, the human-in-the-loop control point is the same primitive everywhere — a local `echo`, a cockpit button, or a write that arrived over the mesh. The principle is [approvals as files](/concepts/approvals-as-files): the trust boundary is a file you have to write to, not a callback an agent can invoke on itself.

## Engine duality: same files, two engines

`AgentDevice` is generic over a pluggable `AgentEngine` trait (`crates/wanix-agent/src/engine.rs:15-25`). The file surface is identical regardless of which engine backs it; only the bytes flowing through `events`/`reply` differ. Two engines ship:

- **`FakeEngine`** — deterministic. Each prompt produces a fixed `you said: <prompt>` reply, streamed as `message.delta` / `message` / `tokens` / `turn.completed` events, with no network and no subprocess (`crates/wanix-agent/src/fake.rs:19-175`). This is what the **served** `#agent` uses: `serve --wanix-services` wires the device with `Arc::new(FakeEngine)` unconditionally (`crates/wanix-cli/src/serve/roots.rs:164-166`).
- **`CodexEngine`** — a real `codex app-server` subprocess bridge, used by the **CLI** `wanix agent` path, which defaults to codex (the `--fake` flag opts into the fake; `--world` confines codex to a Wanix world) (`crates/wanix-cli/src/agent.rs:82-91`).

This split is deliberate, not a gap: the real LLM bridge requires unattended subprocess execution and so stays on the local-trust CLI surface. The served device that any 9P client (including a browser) can reach is the deterministic fake. See [FakeEngine vs. codex](/concepts/fakeengine-vs-codex).

## Delegation: routers, remotes, and POST /agent

Because a session is just files, one agent delegating to another is just reading those files. `RemoteEngine` proxies to a remote `#agent` reachable as an ordinary `FileSystem` at `/n/<peer>/#agent`: it reads `new` to allocate, then operates the remote `<id>/prompt`, `<id>/events`, `<id>/reply`, `<id>/ctl` with no transport coupling — loopback `MemFs`, TCP, or QUIC import all drive the same code (`crates/wanix-agent/src/remote.rs:33-168`). `RouterEngine` composes a default route plus named routes, so "run the agent here" and "run it on node A" are the same call with a different route — Plan 9 cpu's "run there, namespace from here," applied to agents (`crates/wanix-agent/src/router.rs:27-83`). This is [agents as operators](/concepts/agents-as-operators) and [send the agent to the data](/concepts/send-agent-to-the-data).

`POST /agent` exposes the served device as a one-shot HTTP endpoint: the request body is the prompt, the handler allocates a session, submits the turn, and returns the normalized JSONL event log. It is **loopback-only** and requires `--wanix-services` (`crates/wanix-cli/src/serve/http/agent.rs:1-44`).

Over the mesh, the blocking `events`/`reply` reads need their own QUIC bidi stream so a long read does not freeze the serial connection. That routing lives in `wanix-mesh`'s `StreamingImportFs`, not in the device — `RemoteEngine` only opens the file and never assumes the read is cheap (`crates/wanix-agent/src/remote.rs:11-19`). See [streaming import fs](/concepts/streaming-import-fs).

## See also

- [Approvals as files](/concepts/approvals-as-files) — the human-in-the-loop gate as a file write.
- [FakeEngine vs. codex](/concepts/fakeengine-vs-codex) — why the served device is deterministic.
- [Agents as operators](/concepts/agents-as-operators) and [send the agent to the data](/concepts/send-agent-to-the-data) — delegation and the cpu idiom.
- [Live stream vs. one-shot](/concepts/live-stream-vs-one-shot) and [the blocking-stream EOF contract](/concepts/blocking-stream-eof-contract) — `events` vs. `reply`.
- [Service devices](/concepts/service-devices) and [devices import for free](/concepts/devices-import-for-free) — why `#agent` meshes with no agent-specific code.
- [Repair a broken qjs task](/recipes/01-repair-broken-qjs) and [two agents collaborate](/recipes/05-two-agents-collaborate) — the device in a flow.

## Status / honest limits

- **The served `#agent` is not a real LLM.** `serve --wanix-services` wires the device with `FakeEngine`, a deterministic `you said: <prompt>` responder (`crates/wanix-cli/src/serve/roots.rs:164-166`). The real codex bridge is the local-trust `wanix agent` CLI path only (`crates/wanix-cli/src/agent.rs:82-91`).
- **`#agent` is local-trust only.** Like `#task` and `#cpu`, it runs work on behalf of the caller and is not exposed to untrusted or public peers. There are no hard CPU or memory limits yet; this is cheap, scalable isolation, not a sandbox safe for arbitrary untrusted code.
- **`ctl` supports exactly three verbs:** `close`, `approve <req>`, `deny <req>`. Anything else returns `NotSupported` (`crates/wanix-agent/src/files.rs:160-180`).
- **`events`/`reply` are blocking reads.** Over a single 9P connection a blocking read on `events` would stall other operations; the mesh gives blocking opens their own bidi stream via [streaming import fs](/concepts/streaming-import-fs). The default base engine returns `NotSupported` for `wait_reply` and `[]` for `pending` unless an engine overrides them (`crates/wanix-agent/src/engine.rs:58-80`).
