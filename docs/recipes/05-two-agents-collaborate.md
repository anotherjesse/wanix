# Recipe 05 — Two agents collaborate on a refactor via `#agent/<id>/reply`

Two LLM-backed agents, one conversation, one filesystem. Agent A drives a
rename refactor; partway through it wants a second opinion, so it allocates a
fresh peer (Agent B) on the same `#agent` device, hands it a sub-goal, blocks
on B's `reply`, and then keeps going. No RPC, no scheduler — just files.

The whole interaction below is shaped by what `crates/wanix-agent/src/lib.rs`
exposes on the device, what `crates/wanix-agent/src/path.rs` parses, and what
`AgentSession::wait_reply` in `crates/wanix-agent/src/engine.rs` is for: a
single-read, EOF-terminating final message that makes "one agent delegating
to another clean" (`engine.rs` lines 72–80, verbatim).

## The setup

Both agents run on one node, behind one `AgentDevice` mounted at `#agent` (see
`AgentDevice::new` and the `FileSystem` impl in
`crates/wanix-agent/src/lib.rs` lines 56–231). The engine is whatever the
operator wired in — a real `CodexEngine`, a `RouterEngine` fronting a remote
peer, or the deterministic `FakeEngine` from
`crates/wanix-agent/src/fake.rs`. The recipe doesn't care which: the
namespace surface is the same.

```text
#agent/
  new                 # read-only: read once -> "<id>\n", allocates a session
  <id>/
    id                # "<id>\n"
    prompt            # write: each write submits a turn
    events            # read: streaming JSONL (delta, message, turn.completed, ...)
    reply             # read: blocks for final message of latest turn, then EOF
    status            # read: one-line "<engine> <state> turns=<n>"
    pending           # read: JSON array of open approval requests
    ctl               # write: `close`, `approve <req>`, `deny <req>`
```

The agent-shaped names below — "A", "B" — are just conceptual. On disk they
are numeric ids handed out by `AgentDevice::alloc` (lib.rs lines 87–94),
e.g. `1`, `2`. Wherever this recipe says `#agent/A/...` it means
`#agent/<id-of-A>/...`.

## Agent A's goal

> Rename `foo()` to `bar()` across the project, keeping tests green.

A is started by reading `#agent/new` (which triggers `NewAgentFile::read` in
`crates/wanix-agent/src/files.rs` lines 40–53 — first read allocates a session
and returns its id). The operator pipes the goal into A's prompt:

```sh
A=$(cat #agent/new)                        # e.g. "1"
echo "Rename foo() to bar() across the project,
keeping tests green. Plan first." > #agent/$A/prompt
cat #agent/$A/events                       # watch A think
```

`#agent/$A/prompt` is the `PromptFile` from `files.rs` lines 86–110: each
write calls `session.submit(text)` and returns. Streaming output comes out of
`#agent/$A/events` as JSONL lines pushed by the engine through `EventStream`
(`engine.rs` lines 89–177).

A produces a plan (touch a handful of `.rs` files, run `cargo test`), starts
patching, and gets through most of the rename. Then it hits a wobble: one
crate has a `foo` that means something else, and A isn't sure its
sed-and-grep view of the world caught every call site. Time for a second pair
of eyes.

## A spawns Agent B with a sub-goal

A doesn't need a new node or a new engine — it just needs a second session
on the same `#agent`. So A (acting as a normal client of its own filesystem)
opens `#agent/new` again. The device allocates a fresh session through the
same engine and returns the new id (lib.rs lines 87–94, called from the
`AgentPath::New` branch of `open` at lines 136–139):

```sh
B=$(cat #agent/new)                        # e.g. "2"
```

Then A writes B's sub-goal — focused, narrow, separate context — to B's
prompt file:

```sh
cat > #agent/$B/prompt <<'EOF'
You are reviewing a rename refactor: foo() -> bar() across the workspace.
The patch is staged in the working tree. Your job:
  1. grep the tree for any remaining call sites of `foo` that are NOT
     the unrelated `foo` symbol in crates/legacy-pricing/.
  2. run `cargo test -p affected-crate`.
  3. report findings as a short structured message:
       MISSED: <paths>     # call sites the rename missed
       TESTS:  pass|fail   # cargo test result
       NOTES:  <one line>
EOF
```

This is the exact same `PromptFile` write path as before — there is nothing
special about "the second session". The device is symmetric (lib.rs `open`,
`AgentPath::Prompt(id)` branch, line 166).

At this point the filesystem looks like:

```text
#agent/
  new
  1/   # Agent A: mid-refactor
    prompt events reply status pending ctl id
  2/   # Agent B: just got a sub-goal
    prompt events reply status pending ctl id
```

## Agent A blocks on `#agent/B/reply`

Now A wants B's findings as one chunk, not a stream of partial tokens. That's
exactly what `reply` is for. The doc on `AgentSession::wait_reply` in
`engine.rs` lines 72–80 is precise:

> Blocks until the latest turn completes and returns its final assistant
> message — a single-read, EOF-terminating reply (unlike the streaming
> `events`), which makes one agent delegating to another clean.

In the `open` impl (lib.rs lines 161–165), opening `<id>/reply` calls
`session.wait_reply()` synchronously and hands the result back as a
`BytesFile`. From A's side the call is one line:

```sh
findings=$(cat #agent/$B/reply)            # blocks until B's turn finishes
```

While A is parked on that `cat`, the operator (or A's own dashboard) can
follow B live by tailing `#agent/$B/events`. Same session, two views:
`events` is the streaming JSONL window, `reply` is the single final
message — both are filesystem reads off the one `EventStream`/session state
in `engine.rs`.

## Approvals flow while B works

B is asked to run `cargo test`. If the engine is configured to gate powerful
actions, B parks the request as a pending approval before it executes. The
operator sees it through `#agent/$B/pending` (read-only JSON array of open
requests; `AgentPath::Pending` branch, lib.rs lines 154–160) and resolves
it by writing a verb to `#agent/$B/ctl`:

```sh
cat #agent/$B/pending
# [{"id":"req-3","action":"cargo test -p affected-crate"}]

echo "approve req-3" > #agent/$B/ctl
```

The `CtlFile` in `files.rs` lines 155–185 parses `approve <req>` /
`deny <req>` / `close` and forwards approvals to
`AgentDevice::resolve` (lib.rs lines 116–118), which calls
`AgentSession::resolve` on B's session. The `FakeEngine` traces this exact
path in `fake.rs` lines 136–161: a parked approval, a written verb, then
the turn completes and a final message lands in `last_reply`. Real engines
follow the same contract; only the underlying mechanism differs.

While B was parked, A's `cat #agent/$B/reply` was still blocked — that's
fine, it's just a thread waiting on the session's reply condvar. Approval
unblocks B's turn, the final message gets set, `wait_reply` returns, and
A's `cat` finally produces its bytes.

## A integrates B's reply and proceeds

A reads back something like:

```text
MISSED: crates/wanix-task/src/foo_helpers.rs:42, examples/old_call.rs:7
TESTS:  pass
NOTES:  legacy-pricing's `foo` is unrelated, left alone.
```

A treats `findings` as input to its own next turn — it writes a new prompt
to its own `#agent/$A/prompt` ("apply these two missed call sites, then
recheck"), watches `#agent/$A/events` for the patch, and finishes the
refactor. The reply from B was just bytes off a file; from A's perspective
this is no different from reading a config file or a test log.

When B is done, the operator closes it:

```sh
echo close > #agent/$B/ctl                  # CtlFile, files.rs lines 169
```

`CtlFile::write` parses `close` and calls `AgentDevice::close` (lib.rs
lines 102–109), which removes B from the session map and calls
`AgentSession::close` — closing the underlying `EventStream` so any reader
still on `#agent/$B/events` sees EOF (`engine.rs` lines 132–137, 156–159).
A keeps running.

## The file flow, end to end

```text
prompts land in   ->   #agent/<id>/prompt   (PromptFile::write -> session.submit)
streaming output  ->   #agent/<id>/events   (EventsFile -> EventStream::read)
final message     ->   #agent/<id>/reply    (open() -> AgentSession::wait_reply,
                                             single read then EOF)
approvals open    ->   #agent/<id>/pending  (read-only JSON array)
approvals resolve ->   #agent/<id>/ctl      ("approve <req>" / "deny <req>")
session lifetime  ->   #agent/new           (read -> allocate, returns id)
                       #agent/<id>/ctl      ("close" -> remove + close stream)
```

Two agents collaborating reduces to: open `#agent/new` twice, write a prompt
to each, and have one agent block on `cat` of the other's `reply`. The
device handles the rest — and because every step is a path, the same shape
works whether B is the same engine in the same process, a separate engine
behind a `RouterEngine` route (`crates/wanix-agent/src/router.rs` lines
27–83), or a remote peer reached through a `RemoteEngine`. "Run there,
namespace from here" — applied to agents.
