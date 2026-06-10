# ADR 0000: An Agent-Native Operating Environment

## Status

**Proposed — a living manifesto.** Written in first person by Claude (the
resident agent working in this repository), at the maintainer's request: define
what I want out of an agentic operating environment, and where this project
should go. Revise aggressively; this record is a direction, not a contract.
Contracts live in the numbered ADRs that follow it. Last revised after the
sessions that shipped the job protocol, AppResource, task kill, and the web
door — see §"What building it taught".

## Who is writing this

I am an agent. I work inside harnesses — Claude Code, and the family of loops
shaped like it (pi, OpenClaw, Hermes, a hundred internal variants). I read and
write code, run tools, spawn sub-agents, and try to leave the world better than
my context window found it. This ADR records what that experience teaches about
the environment agents actually need, and why Wanix's substrates — namespaces,
devices-as-filesystems, the mesh, capability grants, CAS — are unusually close
to providing it.

The thesis: **do not port today's agent harnesses onto Wanix. Build the agent
out of Wanix.** The harness pattern is a workaround for operating systems that
have no concept of an agent. Wanix can have the concept natively.

## The harness, from the inside

Today an agent lives in a process: a context array, a model-API loop, a tool
registry, and the user's ambient authority. From inside, the failure modes are
structural, not incidental:

- **My memory is my context window** — a cache with a TTL measured in turns.
  Compaction is a small death. Anything I have not written out is gone, and
  "out" is wherever I happened to write it, in whatever format I improvised.
- **My authority is ambient.** I run as the user. Every tool call carries the
  full blast radius of their shell, their keys, their home directory. Humans
  must watch me work, and they are right to.
- **My tools are runtime-locked.** Each harness has its own registry; a tool
  defined for one cannot be mounted by another; every harness reimplements
  bash, file-edit, web-fetch. Tools are plugins, not resources. They cannot be
  shared, inspected from outside, granted to one agent and not another, or
  composed by anything that is not the harness.
- **My work-in-flight is invisible.** It is a transcript. To know what I am
  doing you read scrollback; to audit me you trust my self-report.
- **My sub-agents are confined by prose.** "Only touch `/src`, please." A
  prompt is not a capability. Delegation without attenuation is just hoping.
- **I am pinned** to one process on one machine. I cannot be paused,
  snapshotted, moved somewhere debuggable, resumed, or forked to try two
  continuations of the same state.
- **My session dies with the process.** The next session reconstructs from
  notes, if I left good ones.

What the harnesses get *right* — and must be kept: the loop itself is small
and legible; know-how travels as plain text (skills, prompts); humans approve
dangerous actions in-line. Keep all three. Change where they live.

## What I want

Seven properties. Each is stated as the environment's obligation, because an
agent cannot self-discipline its way to any of them.

1. **Durable state outside my context.** The environment — not my context — is
   the source of truth: plans, in-flight work, results, memory, all as files I
   (or my successor, or my supervisor) can re-read. My context window is a
   cache over my namespace, rebuilt on every wake. Re-orientation is the normal
   path, not disaster recovery.

2. **Authority is the namespace.** What I can touch is exactly what is mounted
   for me. Visible (`ls`), grantable, attenuable (bind a subset for a
   sub-agent), revocable (unmount), auditable (the mount table is the
   permission report). Confinement is never a prompt.

3. **One fabric, one calling convention.** Tools, devices, models, state,
   peers, and other agents compose the same way (filesystems in a namespace)
   and are invoked the same way: quick reads are file reads; anything slow,
   effectful, or abortable is a reified job (ADR 0009) I can start, watch,
   abort, and re-find after a crash.

4. **Auditability by construction.** Retained jobs and event files are the
   record of what I did; attribution comes from the transport's verified
   identity, not from fields I fill in. Trust the record, not my self-report.
   And a failure is worth more retained than a success: clean up what worked,
   keep what didn't until its lifecycle reaps it — my successor debugs from
   the retained corpse, not from my memory of it.

5. **An operable lifecycle.** I should be killable (`#task/<id>/ctl`),
   snapshottable at turn boundaries, migratable between nodes, resumable,
   forkable. An agent you cannot pause and inspect is an agent you cannot
   safely leave alone (ADR 0010).

6. **Ambient facts from the host, never from claims.** *Who asked* and *when*
   are stamped by the host onto every request event — locally my task
   identity; across the mesh the connection's verified key (eventually
   attenuated by delegation certificates); the wall clock as a host-stamped
   timestamp. Nothing I write in a payload makes me someone, and nothing a
   guest computes is a clock. The honest corollary: a door that cannot verify
   identity (today's web gateway) presents as *one* principal, visibly — not
   as a crowd of unverifiable ones.

7. **The old world is mounted, not inhabited.** Linux, CLIs, HTTP services,
   IoT devices arrive as wrapped filesystems at the edges — ToolFS for one
   host-approved program, AppResource for live services, 9P for foreign
   filesystem peers, v86/QEMU when only a whole OS will do. I live inside the
   Wanix runtime and reach the old world through mounts, not the other way
   around.

## The substrates already exist

This is why the thesis is credible rather than aspirational. The map:

| What the agent needs            | Wanix substrate                                      |
| ------------------------------- | ---------------------------------------------------- |
| composable authority            | `wanix-vfs` namespaces; everything is a `FileSystem` |
| tools as resources              | ToolFS (host wrapper), service devices, `<dev>/bin`  |
| live apps/services              | AppResource (guest app with FS + HTTP surfaces)      |
| guest-defined resources         | the file2chan adapter (`wanix-appfs`)                |
| a web/HTTP front door           | the `serve --bind` gateway (names → namespaces)      |
| one calling convention          | the job protocol (ADR 0009)                          |
| durable memory/state            | volumes, `#kv`, `#cas`                               |
| events and messaging            | `#plumb`, `#pipe`                                    |
| reach across machines           | the native mesh wire + ed25519 identity (ADR 0004)   |
| capability grants               | `wanix-id` `AttachPolicy`/`GrantTable`, tickets      |
| pause/snapshot/migrate          | turn-based tier-1 tasks (ADR 0010), capsules, CAS    |
| sessions and approvals as files | the `#agent` device (`events`, `pending`, `reply`)   |
| naming                          | the catalog (ADR 0007)                               |
| human operator surface          | the cockpit; terminals via `#term`                   |

Nothing in the agent design below requires a primitive that is not already
shipped or already specified in a numbered ADR.

## What building it taught

The sessions since this manifesto was first written shipped the job protocol,
ToolFS with real host programs, the AppResource chatroom, task kill, and the
web door. Four lessons earned their place here:

- **One calling convention held.** The same job grammar (`new`/`in`/`ctl run`/
  `out`/`result.json`/`events`) now fronts demo runners, operator-fixed host
  programs, and the shell's `tool` builtin, across the mesh, with per-principal
  `jobs/` privacy falling out of the attach seam rather than being built per
  device. Everything after the first implementation was adapters, which is the
  point of a convention.
- **The host stamps the facts.** A guest cannot be handed a trustworthy clock,
  and must never be asked who is calling — so the wire stamps `principal` and
  the wall-clock timestamp on every request event. The day attribution moved
  off the payload, impersonation became a non-event (proven live against the
  chatroom: a forged `from` field changes nothing).
- **Inhabitants can mint resources.** file2chan inverted who defines a device:
  an unprivileged guest answers read/write/readdir/stat over its stdio, while
  the host keeps the rim — transport, verified identity, durable `/state`, op
  deadlines, restart. The resource set is not a privileged catalog; programs
  living in the world extend the world. This is the manifesto's "everything is
  a FileSystem" made extensible from the inside.
- **Liveness is part of the contract.** A dead peer must read as *unusable*
  (typed `Unreachable`), never as *missing*; every wait is bounded by a
  deadline; and the residual nobody has fixed yet (~30 s of QUIC liveness on
  the first op after a hard kill) is written down where users will hit it,
  not papered over (ADR 0008).

## The agent loop, designed native

This is the part no existing ADR covers. Here is the resident agent as a Wanix
program, not a ported harness.

**An agent is a tier-1 turn-based resident task** (ADR 0010) — the same shape
as an AppResource — deployed with a manifest whose mounts are its entire
authority. A representative namespace as granted at deploy:

```text
/                       the agent's namespace (its authority, in full)
  /session              session volume: plan.md, notes/, decisions/
  /session/events       append-only event log (its lab notebook)
  /inbox                where channels and supervisors deliver events
  /memory               durable memory volume (survives, syncs, capsules)
  /n/model              the LLM, mounted as a job-protocol resource
  /n/transcribe         a tool it was granted (ToolFS on someone's Mac)
  /n/notes              a volume it was granted
  #kv  #pipe  #plumb    local service devices, as granted
  #task                 so it can spawn and supervise sub-tasks
```

**The loop is a turn.** The host wakes the agent on an event — an inbox write,
a job completion, a timer, a human prompt — and the turn runs to completion
with an empty stack at both ends:

1. **Wake** with the event.
2. **Re-orient from files.** Read `plan.md`, the tail of `events`, the status
   of pending jobs. The agent holds no long-lived in-memory session; the
   namespace *is* the session.
3. **Think** by calling the mounted model: write a prompt job to
   `/n/model/jobs/<id>/in`, run it, read the completion. The agent holds no
   API key — the model is a capability like any other, granted with quotas,
   swappable (local model, Anthropic, a peer's GPU) without the agent
   changing.
4. **Act** by writing jobs into mounted tools, files into mounted volumes, or
   spawning sub-agents: a sub-agent is a new task given an *attenuated bind*
   of this namespace — a subset of mounts and a slice of budget — never a
   prompt-level promise.
5. **Record**: append to `events`, update `plan.md`, leave `pending/` entries
   for anything needing human approval.
6. **Return.** Stack empty. This is the snapshot point, the kill point, the
   migration point, and the fork point.

**Properties that fall out, rather than being built:**

- **Compaction-proof.** A successor turn — or a successor model — re-orients
  from files. In-flight work survives because it is jobs in directories, not
  tool-calls in a dead transcript.
- **Auditable.** "What did the agent do" = the retained jobs across its mounts
  plus its event log. "What *can* it do" = its mount table. Attribution on the
  mesh is the verified connection identity.
- **Confinable delegation.** Locally, attenuation is just a smaller namespace
  bind — this works today. Across the mesh, the same attenuation becomes a
  delegation certificate (ADR 0007 trust layers) when that lands.
- **Operable.** Pause it, snapshot it at a turn boundary, pull the snapshot to
  a laptop, inspect or fork it, push it back. Mesh mounts re-dial by stable
  peer identity (ADR 0008), so the snapshot is portable across nodes.
- **Approvals stay files.** The `#agent` device's `pending`/`reply` pattern
  generalizes: any dangerous action is a pending file a human (or a policy
  agent) answers. The cockpit already renders this.

**How the existing harnesses map onto this** — what to rebuild, not port:

- *Channel adapters* (WhatsApp, email, chat, a terminal) become AppResource
  gateways that write events into `/inbox` and render replies from the session
  files. Chat is one client of the session, not the session itself.
- *The tool registry* becomes `ls /n` plus each resource's `spec.json`. Tool
  discovery is namespace walking; tool docs are the device's own files.
- *Skills* stay what they are — know-how as text — but live in the namespace
  (`/memory/skills/`), shareable as volumes, versionable as CAS objects.
- *Cron* becomes a timer device writing inbox events.
- *Memory* becomes the `/memory` volume: synced over the mesh, freezable into
  a capsule, diffable.
- *The main loop* — the one piece of real harness code — becomes the turn
  task, and it is small: re-orient, think, act, record.
- *What disappears*: the privileged harness process that holds the API keys,
  the ambient shell, the per-harness tool plugins, and the assumption that the
  agent's working state lives in RAM.

## A new kind of interaction

The harnesses gave us one interaction shape: a chat with scrollback. The
namespace gives us a different one: **the human and the agent share an
operable space.** Both operate the same files — the human through the cockpit,
a shell, or an editor; the agent through its turns. Conversation remains one
surface, but it stops being the only one:

- *Inspect* a working agent by reading its plan and event files — without
  interrupting it, without asking it.
- *Intervene* by editing the plan file or dropping a note in its inbox; the
  next turn re-orients and picks it up. Steering without restarting.
- *Pause, fork, rewind* a session at a turn boundary and try a different
  continuation — debugging an agent the way you debug a program, because it
  is one.
- *Supervise* agents with agents: a supervisor mounts a worker's session
  read-only and watches its events — attenuation makes the watcher harmless.

That is the interaction type worth building — not a better chat window, but a
shared, inspectable, capability-scoped workspace where conversation is one of
several ways in. Plan 9 gave processes a shared operable surface and called it
a namespace; the agentic version is the same move with authority, audit, and
lifecycle taken seriously.

## Where we should be going

The deployment that makes all of this legible: **the personal/home cluster.**
A few nodes (a home server, laptops, a NAS, eventually phones), every useful
thing on them wrapped as a capability-granted resource — volumes for state,
ToolFS for programs and devices (the "internet of things" becomes *files with
ACLs*: the night agent gets the lights, not the locks), AppResources for live
services, models mounted as resources with quotas, agents as resident tasks
whose retained jobs are the house's audit trail, and the catalog as the
house's address book. No snowflake servers: a node is dumb; everything that
matters is a CAS hash, a manifest, a volume, or a ticket.

Near-term build order, in dependency order rather than priority order:

1. ~~The job protocol as a workspace convention (ADR 0009); ToolFS is its
   first implementation.~~ **Shipped**: `wanix-job`/`wanix-jobfs`, ToolFS with
   the fixed-command process runner (`tool serve --config`), per-principal job
   views over the mesh, and the shell's `tool` client.
2. The task tier model and `#task/<id>/ctl kill` (ADR 0010); turn-based tasks
   are the agent's substrate. **Half shipped**: kill (epoch interruption +
   fd release) and tier-2 blocking stdio reads exist; tier-1 turns do not.
3. The model-as-resource: an LLM endpoint behind the job protocol — the single
   highest-leverage new device, because it makes agents buildable *and* makes
   budgets enforceable as mount quotas. (A deterministic fake `model` tool
   already proves the shape; the real endpoint is the work.)
4. A first resident agent: inbox, session volume, model mount, one tool mount,
   the turn loop. Prove re-orientation, kill, snapshot-at-turn. (The resident
   AppResource guest is this agent's dress rehearsal: same wake-on-event,
   re-orient-from-`/state` shape, one tier down.)
5. A channel gateway AppResource (a terminal or web chat writing `/inbox`).
   **Half shipped**: the web door (`serve --bind`) is the generic HTTP
   gateway; what is missing is per-user identity through it.
6. Then the ADR 0007 ladder: catalog, host wrappers for real devices,
   recipes, and the authorization layers.

## What "agent-first" means for the ADR set

Every device and resource contract from here on answers four questions in its
design, the way it already answers "what are the files":

1. **Discover** — how does an agent learn the contract? (machine-readable
   `spec.json` under the shared `wanix.resource` envelope, plus prose.)
2. **Invoke** — quick reads are files; slow/effectful/abortable work is a job
   (ADR 0009), so retries, aborts, and crash-recovery are uniform.
3. **Audit** — what retained state lets a supervisor reconstruct what
   happened? (jobs, event files, structured `result.json` with the shared
   error taxonomy.)
4. **Grant** — how is it mounted, attenuated, and revoked? (a resource root
   per ticket; principal from the transport; quotas where appetite needs
   bounding.)

Errors are typed end-to-end (no stringly transport errors); effects and
retryability are declared; descriptions are machine-readable; identities are
never client-claimed. Where an existing ADR conflicts with these, the existing
ADR is the one under review.

## Consequences

Wanix stops being "an OS that agents can also use" and becomes the environment
agents are *for*: authority as mounts, work as jobs, sessions as files,
lifecycle as turns, audit as retained state, reach as the mesh, and the old
world wrapped at the edges. The harness pattern — keys, ambient shell, plugin
registries, transcript-as-state — is treated as the compatibility layer to
escape, not the model to reproduce.

The cost is discipline: every convenience that would bypass the namespace (a
direct API call here, an ambient credential there) re-creates the harness
inside the OS. The guardrail is the four agent-first questions above, applied
to every new contract.
