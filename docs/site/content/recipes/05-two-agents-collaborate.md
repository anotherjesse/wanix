---
title: "Recipe 05 — Two Agents Collaborate via #agent/<id>/reply"
slug: recipes/05-two-agents-collaborate
pageType: use-case
oneLiner: "Agent A drives a rename refactor, spawns a second session B with a sub-goal, blocks on B's reply (single-read, EOF-terminating), and integrates the findings — all as files."
audience: [developer]
tags: [mesh, agent, local-trust-only, caveat, cli]
sourceRefs:
  - docs/recipes/05-two-agents-collaborate.md
  - crates/wanix-agent/src/engine.rs:71-80
  - crates/wanix-agent/src/router.rs:27-83
  - crates/wanix-agent/src/lib.rs:161-166
  - crates/wanix-agent/src/remote.rs:160
seeAlso:
  - devices/agent
  - concepts/approvals-as-files
  - concepts/send-agent-to-the-data
  - devices/plumb
  - concepts/best-effort-epidemic-delivery
prerequisites:
  - devices/agent
  - concepts/approvals-as-files
  - concepts/send-agent-to-the-data
usedInFlows:
  - {flow: learn/agent-on-your-files, step: 4}
honestLimits:
  - "The served #agent is a deterministic FakeEngine, not a live LLM; real codex is the local-trust 'wanix agent' CLI path only."
  - "Exec devices (#task/#agent/#cpu) are local-trust only; never exposed to untrusted or public peers, and there are no hard CPU/memory limits yet."
  - "serve handles one 9P frame at a time per connection, so a blocking reply or #plumb recv can't interleave with a write on the same connection."
canonicalCaveatFor: []
---

# Recipe 05 — Two Agents Collaborate via `#agent/<id>/reply`

Agent A drives a rename refactor, spawns a second session B with a sub-goal, blocks on B's reply (single-read, EOF-terminating), and integrates the findings — all as files.

**What & why.** You want one agent to delegate a focused sub-task to a second agent, get one clean answer back, and keep going — without an RPC framework, a message bus, or a scheduler. In Wanix an agent is just a directory tree under `#agent`, so "two agents collaborating" reduces to opening `#agent/new` twice and having one agent `cat` the other's `reply` file. The device handles concurrency, blocking, and lifetime; the collaboration protocol is the filesystem.

## The setup: two sessions behind one device

Build the CLI, then start a node that serves the agent device — `--wanix-services` binds `#agent`, and the `--p9` door gives host-side 9P clients something to dial:

```sh
cargo build --locked --package wanix-cli       # see /reference/build-and-install
alias wanix='./target/debug/wanix'

mkdir -p /tmp/two-agents
wanix serve --root /tmp/two-agents --listen 127.0.0.1:7654 \
  --p9 127.0.0.1:7664 --wanix-services
```

One framing note before the transcript: the `cat`/`echo` lines below are **namespace operations** — they read as a shell *inside* the Wanix world (which is exactly how an agent, itself a task in that world, drives a sibling session). From your host shell, each `cat FILE` is `wanix mount-cat tcp://127.0.0.1:7664 FILE` and each `echo TEXT > FILE` is `wanix mount-write tcp://127.0.0.1:7664 FILE "TEXT"` — [recipe 01 §4](/recipes/01-repair-broken-qjs) shows that form end to end.

Both agents run on one node behind one `AgentDevice` mounted at `#agent`. Each session is allocated lazily — the first read of `#agent/new` hands back a numeric id and creates the session. The file tree per session is:

```text
#agent/
  new                 # read once -> "<id>\n", allocates a session
  <id>/
    prompt            # write: each write submits a turn
    events            # read: streaming JSONL (delta, message, turn.completed)
    reply             # read: blocks for the final message of the latest turn, then EOF
    status            # read: "<engine> <state> turns=<n>"
    pending           # read: JSON array of open approval requests
    ctl               # write: close | approve <req> | deny <req>
    id                # "<id>\n"
```

The agent-shaped names "A" and "B" below are conceptual; on disk they are numeric ids (`1`, `2`). Wherever this recipe writes `#agent/A`, read `#agent/<id-of-A>`.

## Agent A's goal and the prompt

A's goal: *rename `foo()` to `bar()` across the project, keeping tests green.* You allocate A by reading `#agent/new`, then pipe the goal into its prompt:

```sh
A=$(cat #agent/new)                        # e.g. "1"
echo "Rename foo() to bar() across the project,
keeping tests green. Plan first." > #agent/$A/prompt
cat #agent/$A/events                       # watch A think
```

Each write to `#agent/$A/prompt` calls `session.submit(text)`; streaming tokens come out of `#agent/$A/events` as JSONL. A produces a plan, patches a handful of `.rs` files, and gets most of the way through the rename — then hits a wobble. One crate has an unrelated `foo` symbol, and A is not sure its grep view caught every real call site. Time for a second pair of eyes.

## A spawns Agent B with a sub-goal

A needs no new node and no new engine — just a second session on the same device. Acting as a normal client of its own filesystem, A opens `#agent/new` again, and the device allocates a fresh session through the same engine:

```sh
B=$(cat #agent/new)                        # e.g. "2"
cat > #agent/$B/prompt <<'EOF'
You are reviewing a rename refactor: foo() -> bar() across the workspace.
Your job:
  1. grep the tree for remaining call sites of `foo` that are NOT the
     unrelated `foo` symbol in crates/legacy-pricing/.
  2. run `cargo test -p affected-crate`.
  3. report: MISSED <paths> / TESTS pass|fail / NOTES <one line>
EOF
```

This is the same `PromptFile` write path as A's; the device is symmetric. There is nothing special about "the second session."

## A blocks on `cat #agent/B/reply`

A wants B's findings as one chunk, not a token stream. That is exactly what `reply` is for. The contract is documented verbatim on `AgentSession::wait_reply` (`crates/wanix-agent/src/engine.rs:71-80`): it blocks until the latest turn completes and returns its final assistant message — a *single-read, EOF-terminating* reply, unlike the streaming `events`, which is what makes one agent delegating to another clean. Opening `<id>/reply` calls `session.wait_reply()` synchronously (`crates/wanix-agent/src/lib.rs:161-166`):

```sh
findings=$(cat #agent/$B/reply)            # blocks until B's turn finishes
```

While A is parked on that `cat`, you can follow B live by tailing `#agent/$B/events`. Same session, two views off the one `EventStream`: `events` is the streaming window, `reply` is the single final message. This is the Plan 9 move named: *delegation as a blocking read of a file.*

## Approvals flow while B works

B is asked to run `cargo test`. If the engine gates powerful actions, B parks the request as a pending approval before it executes. You see it through `#agent/$B/pending` and resolve it by writing a verb to `#agent/$B/ctl`:

```sh
cat #agent/$B/pending
# [{"id":"req-3","action":"cargo test -p affected-crate"}]
echo "approve req-3" > #agent/$B/ctl
```

`ctl` parses `approve <req>` / `deny <req>` / `close` and forwards approvals to the session. While B was parked, A's `cat #agent/$B/reply` was still blocked — that is fine; it is a thread waiting on the session's reply condition. Approval unblocks B's turn, the final message is set, `wait_reply` returns, and A's `cat` finally produces bytes. The approval *is* a file write; there is no out-of-band approval channel. See [approvals as files](/concepts/approvals-as-files).

## A integrates B's reply and proceeds

A reads back something like:

```text
MISSED: crates/wanix-task/src/foo_helpers.rs:42, examples/old_call.rs:7
TESTS:  pass
NOTES:  legacy-pricing's `foo` is unrelated, left alone.
```

A treats `findings` as input to its own next turn — it writes a new prompt to `#agent/$A/prompt` ("apply these two missed call sites, then recheck"), watches its own `events`, and finishes. From A's perspective, B's reply was no different from reading a test log. When B is done, close it:

```sh
echo close > #agent/$B/ctl
```

`close` removes B from the session map and closes its `EventStream`, so any reader still on `#agent/$B/events` sees EOF. A keeps running.

## The file flow, end to end

```text
allocate          ->  #agent/new            (read -> "<id>\n")
submit a turn     ->  #agent/<id>/prompt    (write -> session.submit)
streaming output  ->  #agent/<id>/events    (EventStream::read)
final message     ->  #agent/<id>/reply     (open -> wait_reply, single read then EOF)
approvals open    ->  #agent/<id>/pending   (read-only JSON array)
approvals resolve ->  #agent/<id>/ctl       (approve <req> / deny <req>)
teardown          ->  #agent/<id>/ctl       (close -> remove + EOF the stream)
```

Two agents collaborating is: open `#agent/new` twice, write a prompt to each, and have one agent block on `cat` of the other's `reply`. Because every step is a path, the same shape works whether B is the same engine in this process, a separate engine behind a `RouterEngine` route (`crates/wanix-agent/src/router.rs:27-83`), or a remote peer through a `RemoteEngine` (`crates/wanix-agent/src/remote.rs:160`). The router's bare `start_session` hits the default route; `start_session_on(name)` targets a named route — "run the agent here" and "run it on node A" are the same call with a different route. That is Plan 9 cpu's "run there, namespace from here," applied to agents.

## See also

- [#agent device](/devices/agent)
- [Approvals as files](/concepts/approvals-as-files)
- [Send the agent to the data](/concepts/send-agent-to-the-data)
- [#plumb device](/devices/plumb) · [Best-effort epidemic delivery](/concepts/best-effort-epidemic-delivery)
- [Agent on your files](/learn/agent-on-your-files)

## Status / honest limits

- **The served `#agent` is a deterministic `FakeEngine`, not a live LLM.** Over `serve`, the device is backed by a scripted `FakeEngine` whose approvals and replies are deterministic — useful for proving the file flow, not for real reasoning. A real codex-backed engine runs only on the local-trust `wanix agent` CLI path, in the same process as your files. The recipe's shapes are identical across engines; only the intelligence behind `prompt`/`reply` differs.
- **`#agent` is local-trust only.** `#agent` (like `#task` and `#cpu`) executes code and is never exposed to untrusted or public peers. It offers cheap, scalable isolation, not a sandbox safe for arbitrary untrusted code, and there are no hard CPU or memory limits yet.
- **Live blocking has a single-frame caveat.** `serve` processes one 9P frame at a time per connection, so a blocking `cat #agent/B/reply` (or a `#plumb` recv) cannot interleave with a write on the same connection. When A and B are driven over one served connection, run the blocking read on its own connection, or drive the collaboration from the in-process CLI path where each `cat` is its own thread.
- **`RouterEngine`/`RemoteEngine` routing works; cross-machine collaboration still rides the mesh trust boundary.** A remote route is reached through an imported peer `#agent`, gated by a default-deny capability bind on the verified peer key. Per-peer namespace paths are a labelled convention, not a shipped per-peer mount.
