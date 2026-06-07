---
title: Approvals as Files (the Trust Gate)
slug: concepts/approvals-as-files
pageType: concept
oneLiner: Powerful actions park in #agent/<id>/pending; nothing runs until a human writes approve <req> to #agent/<id>/ctl.
audience: [newcomer, developer, visionary]
tags: [agent, mesh, local-trust-only, caveat, shipped]
sourceRefs:
  - crates/wanix-agent/src/files.rs:155-185
  - crates/wanix-agent/src/lib.rs:111-118
  - crates/wanix-agent/src/lib.rs:154-159
  - crates/wanix-agent/src/fake.rs:79-161
  - crates/wanix-agent/src/path.rs:18-46
  - docs/recipes/01-repair-broken-qjs.md:226-234
seeAlso:
  - devices/agent
  - concepts/fakeengine-vs-codex
  - concepts/agents-as-operators
  - concepts/traceable-namespaces
prerequisites:
  - devices/agent
usedInFlows:
  - {flow: agent-on-your-files, step: 4}
honestLimits:
  - "ctl supports only close, approve, and deny; any other verb returns NotSupported."
  - "The served #agent runs a deterministic FakeEngine, not a live LLM; its approve:-prefixed prompt is a fixture for exercising the gate, not a model deciding to act."
  - "#agent is an exec device and is local-trust only; it is not exposed to untrusted or public peers."
canonicalCaveatFor: [approvals-are-files]
---

# Approvals as Files (the Trust Gate)

Powerful actions park in `#agent/<id>/pending`; nothing runs until a human writes `approve <req>` to `#agent/<id>/ctl`.

An agent that can edit files and run commands is only as trustworthy as the moment it asks permission. Wanix puts that moment where everything else already lives: in the filesystem. The agent does not call an approval callback or raise a UI modal. It writes an approval request into a file you can `cat`, and it blocks. You grant or refuse by `echo`-ing a verb into another file. The trust boundary is not a hidden code path — it is a file, readable by any client over 9P.

## Show it: an action parks, then a human releases it

Drive a session the way the agent device exposes it — every step is a read or a write on a file (`crates/wanix-agent/src/path.rs:18-46`):

```sh
id=$(cat '#agent/new')

# Ask for something that needs a powerful action.
echo 'approve: write out/result.txt' > "#agent/$id/prompt"

# The turn does not complete. It parks. Read what is waiting:
cat "#agent/$id/pending"
# -> [{"id":"req-1","action":"write out/result.txt"}]

cat "#agent/$id/status"
# -> fake awaiting-approval=req-1 turns=1
```

Nothing has been written to `out/result.txt`. The turn is suspended on a human decision. You release it by naming the request and a verb:

```sh
echo 'approve req-1' > "#agent/$id/ctl"

cat "#agent/$id/pending"
# -> []
cat "#agent/$id/reply"
# -> approved: write out/result.txt
```

`echo 'deny req-1'` instead resolves the same request as declined and unblocks the turn without performing the action. The Plan 9 name for what you just did: the *control file*. `ctl` is a write-only verb channel, the same shape Plan 9 devices have used for decades — a file whose contents are commands, not data.

## Resolution is a three-hop file write

The `approve req-1` you wrote is not parsed by an HTTP handler. It is the `write` method of an open file, and it threads through exactly three hops.

First, `CtlFile::write` collects the bytes, trims them, and splits the verb from its argument (`crates/wanix-agent/src/files.rs:155-185`):

```rust
match verb {
    "close" => self.device.close(&self.id)?,
    "approve" | "deny" => {
        let request_id = parts.next().ok_or_else(|| {
            FsError::Other("agent ctl: approve/deny require a request id".to_owned())
        })?;
        self.device.resolve(&self.id, request_id, verb)?;
    }
    _ => return Err(FsError::NotSupported),
}
```

A bare `approve` with no request id is an error, not a guess — the gate refuses to act on an ambiguous decision. Second, `AgentDevice::resolve` looks up the session by id and forwards the decision (`crates/wanix-agent/src/lib.rs:111-118`). Third, `AgentSession::resolve` is where the parked action either runs or is discarded.

The session enforces an exact match: it takes the pending request, checks the request id, and on mismatch puts it back and returns `NotFound` (`crates/wanix-agent/src/fake.rs:136-161`):

```rust
match guard.take() {
    Some(p) if p.id == request_id => p.action,
    other => {
        *guard = other;
        return Err(FsError::NotFound);
    }
}
```

You cannot approve a request id that is not pending, and approving one request never silently releases another. The decision (`approve` or `deny`) selects the outcome, an event lands on the stream, and the turn finishes. The `pending` file then reads `[]` because the slot is empty again.

## The FakeEngine makes the gate deterministic

The served `#agent` does not run a live model — it runs `FakeEngine`, a fixture that produces fixed replies with no network and no subprocess (`crates/wanix-agent/src/fake.rs`). To exercise the trust gate without an LLM, it uses a contract: a prompt prefixed `approve:` parks an approval request instead of replying (`crates/wanix-agent/src/fake.rs:79-101`):

```rust
if let Some(action) = prompt.strip_prefix("approve:") {
    let id = format!("req-{turn}");
    *pending = Some(PendingApproval { id, action });
    self.stream.push_line(/* { "t": "approval.needed", "id", "action" } */);
}
```

So `approve: write out/result.txt` is not the model deciding to write a file. It is a test affordance that makes the *gate* behave exactly as the real flow would: park, surface in `pending`, block the turn, wait for `ctl`. The plumbing the cockpit and recipes drive — the part that actually matters for trust — is real and deterministic; only the engine behind it is a fixture. The real-codex path lives behind the local-trust `wanix agent` CLI, not the served device. See [FakeEngine vs codex](/concepts/fakeengine-vs-codex).

## Why approvals as files is the legibility story

Approvals-as-files is the agent half of [traceable namespaces](/concepts/traceable-namespaces). Because the gate is a file, the same read/write contract reaches it from a shell, a script, the browser cockpit, or — over 9P — a remote operator. There is one place to look (`pending`), one place to act (`ctl`), and one observable record (the event stream and `reply`). The agent repair demo leans on exactly this: a powerful edit parks as an approval request, and nothing happens until a human writes `approve <id>` to `ctl` (`docs/recipes/01-repair-broken-qjs.md:226-234`).

This is what makes an agent an *operator* rather than an actor with hidden powers. It can propose; it cannot proceed. The boundary between proposing and proceeding is not buried in a callback — it is a file you can audit, test, and gate on. See [agents as operators](/concepts/agents-as-operators).

## See also

- [The #agent device](/devices/agent) — the full file shape: `new`, `prompt`, `events`, `pending`, `ctl`, `status`, `reply`.
- [FakeEngine vs codex](/concepts/fakeengine-vs-codex) — why the served engine is deterministic and where real codex runs.
- [Agents as operators](/concepts/agents-as-operators) — propose-then-proceed as the agent posture.
- [Traceable namespaces](/concepts/traceable-namespaces) — the broader legibility story this page is one part of.
- [Repair a broken qjs program](/recipes/01-repair-broken-qjs) — the gate in a working demo.
- [Two agents collaborate](/recipes/05-two-agents-collaborate) — approvals across delegated sessions.

## Status / honest limits

- **`ctl` is a small, closed verb set.** It accepts only `close`, `approve`, and `deny`; any other verb returns `FsError::NotSupported` (`crates/wanix-agent/src/files.rs:176`). `approve`/`deny` require a request id or the write fails.
- **The served engine is a fixture, not a model.** The served `#agent` runs `FakeEngine`; its `approve:`-prefixed prompt is how a test parks a request, not a model choosing to act. Real codex is the local-trust `wanix agent` CLI path only.
- **`#agent` is local-trust only.** It is an exec device and is not exposed to untrusted or public peers. The approval gate makes actions legible; it does not make running arbitrary agent-driven commands safe for untrusted callers.
- **One pending slot per session, exact-match resolution.** The fixture session holds a single pending approval; resolving a request id that is not the one pending returns `NotFound` (`crates/wanix-agent/src/fake.rs:142-148`).
