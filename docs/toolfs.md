# ToolFS Plan

Status: **draft design note**. This is not an ADR yet. It sketches the
host-wrapper filesystem from [ADR 0007](adrs/0007-resources-catalogs-and-pairing.md)
and the validation path needed before the contract is promoted.

> Update: the job-directory invocation shape this document introduced has been
> promoted to a workspace convention — [ADR 0009](adrs/0009-job-protocol.md)
> (the job protocol) now owns the grammar, the shared error taxonomy, and the
> job-id-as-idempotency-key rule. ToolFS is its first implementation. `ctl
> abort` bottoms out in `#task/<id>/ctl kill` (ADR 0010).
>
> Update: the v0 core crate exists at `crates/wanix-tool` — `ToolService` +
> principal-scoped `ToolFs` views over the `wanix-job` vocabulary, with the
> contract, privacy, lifecycle/TTL, quota, and abort rows of the validation
> matrix pinned by filesystem-surface tests against the fake/manual/model
> runners. Mesh serving has landed (`wanix tool serve` — one native endpoint
> and ticket per tool, per-connection principal-scoped `jobs/` views from the
> verified `remote_id()`), and so has the client helper, as the pipeable
> wanix-sh `tool` builtin (`tool /n/upper < in > out`). The process runner,
> catalog entries, and the agent adapter remain build slices ahead.

## What

ToolFS is a Wanix filesystem resource that exposes one host-approved operation
family as files.

A host runs a local program such as a formatter, checksum tool, or speech-to-text
binary behind a fixed policy. The host chooses the executable, argv template,
environment, working directory, limits, lifecycle, and output mapping. A remote
caller supplies only input bytes and validated parameters through the mounted
filesystem.

The visible shape is intentionally small:

```text
/n/upper/
  spec.json
  new
  jobs/<job>/
    in
    params.json
    ctl
    out
    err
    status
    result.json
```

One mounted ToolFS root is one bounded capability. One job directory is one
operation attempt.

## Why

Wanix already has the hard part: every resource is a `FileSystem`, namespaces
compose by binding filesystems, and native mesh imports make a remote filesystem
look local. ADR 0007 names the missing product layer: users need resources they
can catalog, mount, and compose by name.

ToolFS is the missing "host wrapper" resource: a safe way to project one local
CLI capability onto the mesh without giving the caller arbitrary remote
execution.

The important inversion:

```text
#cpu:    caller chooses program/argv/env, host runs the job
ToolFS: host chooses program/argv/env, caller supplies data
```

That difference is the trust boundary. If ToolFS lets callers choose an
executable, shell string, argv template, environment, or working directory, it
has collapsed into `#cpu` with a friendlier name.

## ToolFS And Agent Tools

Modern agents already have tool APIs: named functions, JSON schemas, structured
observations, retries, cancellation, and policy. ToolFS should learn from those
interfaces, but it should not become one.

ToolFS is **agent-legible, not agent-native**.

An agent can read mounted ToolFS specs and project them into its own convenient
tool registry:

```text
/n/json-format/spec.json
/n/hash/spec.json
/n/text-stats/spec.json
```

A skill can teach the agent how to use ToolFS. An adapter can offer
`call_toolfs("/n/hash", input, params)`. But that adapter is only convenience
over files. The durable capability is the mounted resource in the namespace.

This keeps the claw open: shells, qjs tasks, wasm tasks, cockpit, humans, and
agents all grasp the same thing. Authority is visible in the namespace instead
of hidden inside one agent runtime.

Useful borrowings from agent tool APIs:

- `spec.json` must be machine-readable.
- Params should have a schema and be validated before `run`.
- `status` and `result.json` should be structured.
- Errors should distinguish invalid input, limits, timeout, abort, and runner
  failure.
- Side effects and retryability should be declared.
- Cancellation should be explicit.

Things ToolFS should not copy:

- A generic function-call protocol inside the filesystem.
- A workflow engine.
- A hidden global tool registry.
- Agent-only auth, identity, or cleanup semantics.
- Arbitrary remote command execution.

## How

The rest of this plan pins the concrete resource shape: how ToolFS is addressed,
what files it exposes, how jobs move through their lifecycle, what privacy means
for multiple principals, where process execution lives, and what validation
should prove before the contract graduates into an ADR.

## Resource Model

Follow ADR 0007's v0 resource rule: **one ticket names one resource root**.

A tool server binary may expose many tools, but it should do so by running one
native mesh endpoint per tool and printing or registering one ticket per tool:

```text
tool server process
  upper       -> iroh://A...?addr=...  root = ToolFS upper
  hash        -> iroh://B...?addr=...  root = ToolFS hash
  json-format -> iroh://C...?addr=...  root = ToolFS json-format
```

Clients compose those tickets into namespaces:

```text
qjs-shell \
  --mount-mesh iroh://A...?addr=...=/n/upper \
  --mount-mesh iroh://B...?addr=...=/n/hash
```

Scoped subresources such as one endpoint with `/services/upper` and
`/services/hash` are a later optimization, not the v0 contract. Per-resource
tickets avoid depending on native attach scoping that does not exist yet, and
they preserve a useful bearer-capability property: a caller with the `upper`
ticket cannot discover sibling tools by changing a path.

## Filesystem Contract

Proposed root:

```text
/
  spec.json              read-only machine contract
  params.schema.json     optional read-only JSON schema
  health                 read-only readiness snapshot
  usage                  read-only caller-visible quota snapshot
  new                    read allocates an opaque job id
  jobs/                  directory of visible jobs for this principal

/jobs/<job>/
  in                     write request bytes before run
  params.json            optional write params before run
  ctl                    write run | abort | close
  out                    primary output bytes
  err                    stderr or diagnostics
  status                 structured state snapshot
  result.json            structured final summary
  events                 progress stream (never-EOF read while the job lives)
```

`events` is implemented: a bounded, lossy (drop-oldest) progress stream the
runner feeds through its `RunContext`; a read blocks for new lines and sees
EOF only once the job reaches a terminal state (or is dropped). A device that
cannot honor it may omit it (ADR 0009).

`new` allocates a job and returns an opaque id:

```sh
id=$(cat /n/upper/new)
```

The caller writes input and optional params, then explicitly starts the job:

```sh
printf 'hello\n' > /n/upper/jobs/$id/in
printf '{"mode":"ascii"}\n' > /n/upper/jobs/$id/params.json
echo run > /n/upper/jobs/$id/ctl
cat /n/upper/jobs/$id/out
cat /n/upper/jobs/$id/result.json
```

Do not make "run when `in` closes" the durable contract. The current Wanix
`File` trait has no explicit close callback, and close semantics become subtle
across transports. `ctl run` is simple and portable: it seals input and starts
the operation.

One-shot helpers can be sugar later:

```sh
tool /n/upper < input.txt > output.txt
```

That helper would allocate, write `in`, write `run`, read `out` and
`result.json`, then write `close`. It must remain a client convenience over the
visible file protocol.

## Spec Shape

`spec.json` is the mounted source of truth. Catalog entries stay small
(`name`, `tags`, `address`, summary); the live filesystem describes the
capability in detail.

Sketch:

```json
{
  "wanix.resource": "v0",
  "kind": "tool",
  "name": "upper",
  "description": "Uppercase UTF-8 text.",
  "input": {
    "mode": "bytes",
    "contentTypes": ["text/plain; charset=utf-8"],
    "maxBytes": 1048576
  },
  "params": {
    "schemaPath": "params.schema.json",
    "required": false
  },
  "outputs": {
    "primary": {
      "path": "out",
      "contentType": "text/plain; charset=utf-8"
    },
    "diagnostics": {
      "path": "err"
    }
  },
  "limits": {
    "runTimeoutMs": 5000,
    "maxConcurrentPerPrincipal": 2,
    "maxJobsPerPrincipal": 32,
    "maxBytesPerPrincipal": 16777216,
    "maxTotalJobs": 1024,
    "maxTotalBytes": 268435456,
    "maxOutBytes": 16777216,
    "maxErrBytes": 1048576
  },
  "lifecycle": {
    "allocatedTtlMs": 300000,
    "retainDoneMs": 600000,
    "retainFailedMs": 3600000
  },
  "visibility": "private",
  "sideEffects": "none",
  "retryable": true
}
```

`params.schemaPath` is **advertisement only**: v0 serves the schema file for
callers to read, but validates `params.json` for JSON well-formedness, never
for schema conformance — runners must treat params as caller input either
way. `visibility` must be `private` in v0: `ToolService` refuses to construct
a spec advertising an unimplemented privacy mode.

Keep this intentionally smaller than a full agent tool protocol. It only needs
to describe the filesystem operation well enough for clients and agents to use
it correctly.

`"wanix.resource"` is the **shared spec envelope**: one versioned outer shape
(`wanix.resource`, `kind`, `name`, `description`, limits/lifecycle/effects)
used by every self-describing resource — ToolFS (`"kind": "tool"`), AppResource
(`"kind": "app"`), and catalog entries that embed a summary of it. Wanix
already has three description surfaces growing independently; the envelope is
decided *now*, before a third dialect ships, so agents parse one outer shape
everywhere (ADR 0000 §agent-first).

## Status And Result

`status` should be structured JSON, even if a shell user can still `cat` it.

States:

```text
allocated -> receiving -> running -> done
                              |-> failed
                              |-> aborted
done/failed/aborted -> retained -> expired
```

Status sketch:

```json
{
  "state": "running",
  "createdAt": 1710000000000,
  "startedAt": 1710000000123,
  "expiresAt": null,
  "inputBytes": 12,
  "outputBytes": 0
}
```

Final result sketch:

```json
{
  "state": "done",
  "exitCode": 0,
  "durationMs": 18,
  "inputBytes": 12,
  "outputBytes": 12,
  "error": null,
  "retryable": true
}
```

Failure result sketch:

```json
{
  "state": "failed",
  "exitCode": 2,
  "durationMs": 4,
  "error": {
    "kind": "invalid_input",
    "message": "input is not valid JSON"
  },
  "retryable": false
}
```

The stable part is the error taxonomy, not the prose — and it is no longer
ToolFS-private: [ADR 0009](adrs/0009-job-protocol.md) owns it workspace-wide:

```text
invalid_params
invalid_input
input_too_large
quota_exceeded
timeout
aborted
runner_failed
unavailable
internal
```

Because the job id is the idempotency key (ADR 0009), a caller that times out
or loses the connection resolves the outcome by re-reading this job's
`status`/`result.json` — never by allocating a new job and re-running blind.
A repeated `run` on the same job is deduplicated.

## Privacy And Principal Views

Default visibility should be `private`.

Two principals mounting the same ToolFS see the same root metadata:

```text
spec.json
params.schema.json
health
usage
new
```

But each sees only their own jobs under `jobs/`. If a caller guesses another
principal's job id, ToolFS should return `NotFound`, not `PermissionDenied`,
unless an explicit operator diagnostic mode is active.

Potential visibility modes:

```text
private   callers see only their own jobs
shared    all authorized callers see one shared job set
operator  callers see their own jobs; owner/operator sees all
public    demo-only, no job privacy boundary
```

The principal must come from the transport or attach layer. Do not accept
client-claimed `user`, `agent_id`, or `owner` fields for privacy, attribution,
or ACL decisions.

Implementation consequence: the core `FileSystem` trait can remain
principal-blind if the server returns a principal-scoped ToolFS view at attach
time. The ToolFS service can hold shared state, while each mounted view carries
the acting principal used for filtering and quotas.

## Lifecycle And Quotas

Lifecycle is not cleanup trivia; it is part of the resource contract. ToolFS
must not grow forever, and the host must know when an operation is done.

Required controls:

```text
allocated TTL              job was created but never run
run timeout                process or fake runner cannot run forever
retain done TTL            successful jobs remain briefly inspectable
retain failed TTL          failed jobs remain longer for debugging
explicit close             caller asks to delete retained job
max jobs per principal     prevent unbounded job directories
max bytes per principal    bound stored input/output/err
max concurrent per principal
global max concurrent
```

`ctl close` removes a retained job. `ctl abort` cancels a running job where the
runner supports cancellation, or marks cancellation requested if process
termination is best-effort.

Do not auto-delete immediately after `out` reaches EOF. A caller may still need
`err`, `status`, or `result.json`, and deleting on read makes debugging and
retries unpleasant.

## Runner Boundary

Split the filesystem contract from host process policy.

Core crates:

```text
wanix-jobfs
  -> wanix-fs + wanix-job + serde + serde_json
wanix-tool
  -> wanix-fs + wanix-job + wanix-jobfs + serde + serde_json
```

The contract is split across two crates: `wanix-jobfs` owns the reusable job
machinery (the job table, lifecycle/quota core, `JobPrincipal`, `JobClock`,
`JobLimits`/`JobLifecycle`, the `JobRunner`/`RunContext`/`RunOutcome` seam,
and the job-directory `File` impls), so the second adopter never depends on a
crate named "tool"; `wanix-tool` owns the tool spec and path layout:

```text
wanix-jobfs                      wanix-tool
  JobCore (table + lifecycle)      ToolService (spec surface)
  JobPrincipal, JobClock           ToolFs (path layout)
  JobLimits, JobLifecycle          ToolSpec, ToolVisibility
  JobRunner / RunContext           fake runner tests
  RunOutcome, job-dir File impls
```

A runner receives a `RunContext` per invocation: the job id, the absolute
deadline (`started_at + runTimeoutMs`; the core finalizes a deadline-ignoring
runner as `timeout` anyway), the job's live abort flag, and the `events`
progress sink.

Output caps (`maxOutBytes`/`maxErrBytes`, plus the aggregate `maxTotalBytes`)
are enforced at finalize: stored bytes are always clamped, and a run that
would otherwise have succeeded records `runner_failed` (per-stream cap) or
`quota_exceeded` (aggregate) instead of silently truncating.

Process execution belongs outside the core at first, probably in `wanix-cli`
or a later `wanix-tool-process` crate:

```text
tool server / wanix-cli
  -> wanix-tool
  -> wanix-vfs
  -> wanix-id
  -> wanix-mesh
  -> catalog/volume plumbing
  -> config parsing
  -> temp dirs and process management
```

The fake runner should land first so the filesystem contract, privacy model,
lifecycle, quotas, and validation can be tested without subprocess flakiness.
The process runner comes after the contract is pinned.

Process runner policy:

- no shell string by default;
- executable chosen by the host config;
- argv template chosen by the host config;
- minimal environment;
- fixed working directory or private temp directory;
- bounded input, output, stderr, runtime, and concurrency;
- no caller-supplied executable, argv template, cwd, or env;
- explicit input mapping (`stdin`, temp file, or named workdir file);
- explicit output mapping (`stdout`, temp file, or named workdir file).

## CLI And Server Shape

Possible local server command:

```sh
wanix tool serve --config tools.toml
```

Possible config sketch:

```toml
[tools.upper]
description = "Uppercase UTF-8 text"
command = "/usr/bin/tr"
args = ["[:lower:]", "[:upper:]"]
input = "stdin"
output = "stdout"
visibility = "private"

[tools.upper.limits]
max_input_bytes = 1048576
run_timeout_ms = 5000
max_concurrent_per_principal = 2

[tools.upper.lifecycle]
allocated_ttl_ms = 300000
retain_done_ms = 600000
retain_failed_ms = 3600000
```

The server should print and eventually register one catalog entry per tool:

```text
upper       iroh://A...?addr=...
json-format iroh://B...?addr=...
hash        iroh://C...?addr=...
```

Open mode is acceptable for the Layer 0 + Layer 1 prototype, but ADR 0007's
warning applies: catalogs spread addresses, so bearer secrecy erodes as the
feature succeeds. Private endpoints and ACLs are a follow-up project, not a
reason to block v0.

## Agent Adapter

An agent adapter should be downstream of ToolFS, not part of the core.

Possible adapter operation:

```text
call_toolfs(path, input_bytes_or_artifact, params) -> output_artifact/result
```

Under the hood:

1. Read `spec.json`.
2. Validate params against `params.schema.json` when present.
3. Read `new`.
4. Write `in`.
5. Write `params.json` if needed.
6. Write `run` to `ctl`.
7. Read `out`, `err`, `status`, and `result.json`.
8. Write `close` when the caller does not request retained debug artifacts.

The adapter can make agent use easy without turning ToolFS into an agent-only
system. Skills remain know-how; ToolFS remains capability.

## Validation Examples

Do not start with Whisper. It is too heavy: model files, install variance,
runtime cost, audio formats, and dependency failures would obscure the ToolFS
contract.

Use tiny examples that each prove one seam:

```text
fake-echo     pure Rust fake runner; deterministic filesystem tests
upper         stdin -> stdout using tr or fake runner
json-format   JSON input + params -> formatted JSON output
text-stats    text input -> counts in result.json
sha256        bytes input -> digest output
tar-list      temp-file input -> stdout via tar -tf {input}
sleep-echo    proves running/status/timeout/abort
fail          proves nonzero exit, err, and failed result shape
```

Validation matrix:

```text
contract shape
  spec.json, new, jobs/<id>, in, ctl, out, err, status, result.json

request/response success
  write input, run, read out/result, close

invalid params
  validation fails before runner starts

invalid input
  runner-independent validation path where possible

failure
  nonzero exit produces err and failed result

timeout
  sleep-echo hits run timeout and records timeout

abort
  ctl abort changes state and prevents further normal completion

privacy
  two principal-scoped views see separate jobs

guessing
  foreign job id returns NotFound

lifecycle
  allocated jobs expire, completed jobs retain then expire, close deletes

quotas
  input byte limit, job count, byte count, and concurrency limits hold;
  aggregate caps (maxTotalJobs/maxTotalBytes) bound the table across ALL
  principals, since fresh dialer identities mint fresh per-principal quotas

mesh
  one tool served as one native resource ticket and mounted by address

composition
  shell/qjs/wasm can use the same mounted resource

agent projection
  agent adapter derives a callable tool from spec.json without special power
```

## Build Slices

1. **Contract doc and tests (shipped).** Pin filesystem tree, JSON shapes,
   states, and error taxonomy in `wanix-tool` tests.
2. **Fake runner (shipped).** Implement a deterministic fake runner for `upper`,
   `json-format`, `sleep-echo`, and `fail` style cases.
3. **Principal-scoped views (shipped).** Add `ToolService::open_view(principal)`
   and prove private job filtering with two views.
4. **Lifecycle and quotas (shipped).** Add TTL cleanup, explicit `close`,
   byte/job limits, and concurrency limits — including the aggregate
   `maxTotalJobs`/`maxTotalBytes` caps, since per-principal quotas alone do not
   bound served-tool memory.
5. **Process runner.** Add fixed-command process execution outside the core
   crate, with stdin/stdout and temp-file mapping.
6. **Tool server (shipped).** `wanix tool serve --tool NAME [--tool NAME ...]`
   serves each ToolFS as its own native endpoint (one ticket and one persisted
   identity per tool) in one process, binding every connection to a
   principal-scoped view from the verified `remote_id()`.
7. **Catalog integration.** Register one catalog entry per served tool once the
   volume/catalog side is ready.
8. **Client helper (shipped).** Landed as the pipeable wanix-sh `tool` builtin
   (`tool /n/upper < in > out`, optional `PARAMS_JSON` second argument) — sugar
   over the visible file protocol inside the shell, rather than a
   `wanix tool call` host subcommand.
9. **Agent adapter.** Teach agents to discover ToolFS specs from their namespace
   and call them through files.

## Non-Goals For V0

- Streaming live bidirectional tools.
- Cockpit live streaming over the single-frame 9P websocket edge.
- Native subresource selection under one endpoint.
- Arbitrary remote execution.
- Agent-only tool protocol.
- A workflow engine.
- Remote volume or tool administration.
- Ownership/delegation ACL UI.

## Open Questions

- Should the stable file be named `spec` or `spec.json`? The latter is clearer
  for tools and editors; the former matches some device style.
- Should `status` be JSON despite the short name, or should it be
  `status.json`? Agents benefit from explicit JSON names.
- Does `jobs/` earn its extra path component, or should jobs live directly under
  the root? `jobs/` keeps root metadata uncluttered and avoids collisions with
  future control files.
- What minimum JSON schema subset is worth supporting for params in v0?
- How should a principal be represented in `wanix-tool` without depending on
  `wanix-id`? Partial answer, decided: as a structured opaque type, **never a
  bare 32-byte pubkey** — delegation certificates (attenuated principals: key
  + caveats) must slot into job privacy filtering and quotas without
  rototilling them (ADR 0004 §Open questions, convergence note). The remaining
  open part is only the concrete type shape.
- Should a failed validation create a retained job with `result.json`, or should
  `ctl run` return an immediate filesystem error? Retained failures are more
  inspectable; immediate errors are simpler for shell use.
- How much process confinement can be done portably before introducing OS-
  specific sandboxing?

## Summary

ToolFS should be a simple Wanix resource: mountable, inspectable, bounded, and
usable by every client that understands files. Agent tools sit above it as a
convenience projection. The core contract is the open claw: a capability in the
namespace, not a clenched integration inside one runtime.

