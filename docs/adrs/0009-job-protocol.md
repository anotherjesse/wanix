# ADR 0009: The Job Protocol — Reified Calls as a Workspace Convention

## Status

**Proposed.** The first implementation target is ToolFS
([docs/toolfs.md](../toolfs.md)), which originated the shape. This ADR promotes
the shape from a ToolFS detail to the workspace calling convention, because
agents should learn one way to invoke slow or effectful work everywhere
(ADR 0000 §"agent-first"). First implementation landed: the vocabulary crate
(`crates/wanix-job` — states/transitions, error taxonomy, report shapes, spec
envelope) and ToolFS v0 (`crates/wanix-tool`, with the docs/toolfs.md
validation matrix pinned in `crates/wanix-tool/src/tests.rs`); runners are
in-process v0. The protocol now crosses the mesh: `wanix tool serve` exports a
ToolFS over the native wire (one endpoint/ticket per tool, per-connection
principal-scoped `jobs/` views — `crates/wanix-cli/src/tool/serve.rs`),
`--mount-mesh` dials it into a task/shell namespace, and the wanix-sh `tool`
builtin drives it end to end in the tested walkthrough (recipe
[06](../site/content/recipes/06-compose-volume-and-tools.md)). The machinery
behind the grammar (job table, lifecycle/quota core, principal/clock types,
the `JobRunner`/`RunContext` seam, and the job-dir `File` impls) now lives in
the shared `crates/wanix-jobfs`, so later adopters depend on it rather than
on ToolFS. The reserved `events` file **shipped** for ToolFS: a bounded lossy
progress stream fed by the runner's `RunContext`, never-EOF until the job is
terminal.

## Context

Every platform grows a calling convention. Today's agent runtimes use ephemeral
RPC (tool calls in a transcript): the call vanishes with the process, cannot be
inspected from outside, cannot be safely retried after an unknown outcome, and
leaves no audit trail except the caller's own log.

ToolFS introduced the alternative: **a call reified as a job directory.**
Arguments are files, invocation is a `ctl` write, the return value is a file,
and the whole call has a name, a lifecycle, and retention. That gives agents
exactly what ephemeral RPC cannot:

- a half-done call **survives caller death** (context compaction, crash,
  handoff) — a successor reads `status` and resumes waiting or aborts;
- retained jobs **are the audit trail** — a supervisor reconstructs what an
  agent did from the record, not from the agent's self-report;
- the job id **is the idempotency key** — ADR 0008's no-replay rule keeps its
  teeth (the platform never replays a maybe-sent mutation) while giving the
  caller a safe *resolution* path: read the job's `status` instead of guessing.

Meanwhile `#agent` (prompt sessions), `#cpu` (exec jobs), and AppResource
(guest apps) are each inventing an invocation dialect. N dialects is the
exact failure the convention exists to prevent.

## Decision

### The grammar

A device that offers jobs exposes, at its resource root:

```text
new                    read allocates a job, returns an opaque id
jobs/<id>/
  in                   write request bytes before run
  params.json          optional structured parameters before run
  ctl                  write: run | abort | close
  out                  primary output bytes
  err                  diagnostics
  status               structured JSON state snapshot (live)
  result.json          structured JSON final summary (retained)
  events               optional progress stream (never-EOF read)
```

States: `allocated → receiving → running → done | failed | aborted`, then
`retained → expired`. `ctl run` seals input and starts; `ctl abort` requests
cancellation; `ctl close` releases a retained job. Do not key behavior off
file close; `run` is explicit.

### When a job is required (the two-tier rule)

Quick, read-only, idempotent operations are plain file reads/writes. Anything
**slow, effectful, or abortable** must be invoked as a job. The property is
required; a device may omit files it cannot honor (e.g. `events`), but it must
not invent a parallel invocation shape for work the job protocol fits.

### Idempotency and the no-replay rule

The job id is the idempotency key:

- `new` is the only operation that allocates; it is trivially retryable.
- A repeated `run` on the same job is deduplicated by the device (idempotent
  accept; second `run` on a running/finished job is a no-op or a typed error).
- After a timeout or transport loss, the caller resolves the outcome by
  reading `status`/`result.json` for that id — never by blind re-submission.
  ADR 0008's rule is unchanged: the *platform* never replays; the *caller* now
  has a deterministic way to find out what happened.

### The shared error taxonomy

Workspace-wide, used by `result.json`, `status`, and any structured error
surface (including future device errors outside jobs):

```text
invalid_params | invalid_input | input_too_large | quota_exceeded
timeout | aborted | runner_failed | unavailable | internal
```

Devices may add device-specific `detail`, never device-specific top-level
kinds. `retryable: bool` accompanies every failure.

### Principals, privacy, lifecycle

- The acting principal comes from the transport/attach layer (ADR 0004), never
  from a payload field. `jobs/` is a per-principal view by default; a foreign
  job id reads as `NotFound`, not `PermissionDenied`.
- Retention, TTLs, and per-principal quotas (jobs, bytes, concurrency) are part
  of the contract, declared in the device's spec, not cleanup trivia.

### Discovery

A job-protocol device declares itself in its spec file under the shared
resource envelope (`"wanix.resource": "v0"`): the protocol version, declared
effects (`none | idempotent | at-most-once`), retryability, limits, and
lifecycle. The spec is the machine-readable half; a prose `doc` file may
accompany it.

### Adopters

- **ToolFS** is the reference implementation and ships first.
- **`#agent`** prompt sessions should converge on this grammar (a prompt is a
  job; `events` already exists; approvals stay separate files).
- **`#cpu`** job control should converge when next touched.
- **AppResource** apps use jobs for any effectful op they expose (a chatroom
  `post` may stay a plain write — fast, idempotent-enough, attributed by
  transport; a moderation action is a job).

## Consequences

Agents learn one calling convention for all slow/effectful work: allocate,
write, run, watch, read, close — uniform across local devices, mesh-mounted
tools, and other agents. Crash recovery, safe retry, cancellation, and audit
fall out of the shape rather than being reimplemented per device and per agent
runtime.

Costs accepted: convention pressure on devices the shape fits imperfectly
(mitigated by the two-tier rule — plain files remain first-class for fast
reads); job-state bookkeeping in every adopting device (mitigated by sharing
one implementation crate when the second adopter lands); and declared effects
are not enforceable — a device can lie about `idempotent` — so effect
declarations are trust statements by the device author, like the rest of its
contract.

Exact file shapes, JSON fields, and state-machine proofs belong in the
implementing crates' tests and in docs/toolfs.md. Update this ADR only when
the grammar, the taxonomy, or the idempotency rule changes.
