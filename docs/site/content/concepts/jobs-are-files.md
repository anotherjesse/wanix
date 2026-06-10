---
title: Jobs Are Files
slug: concepts/jobs-are-files
pageType: concept
oneLiner: A call to slow or effectful work is reified as a job directory — arguments are files, invocation is a ctl write, the result is a file with a name, a lifecycle, and retention (ADR 0009).
audience: [developer, visionary]
tags: [tools, jobs, mesh, shipped, caveat]
sourceRefs:
  - docs/adrs/0009-job-protocol.md
  - docs/toolfs.md
  - crates/wanix-tool/src/lib.rs:1-30
  - crates/wanix-tool/src/service.rs:81
  - crates/wanix-job/src/taxonomy.rs
  - crates/wanix-sh/src/tool.rs:1-21
seeAlso:
  - concepts/everything-is-a-file
  - concepts/approvals-as-files
  - concepts/devices-import-for-free
  - concepts/guest-defined-resources
  - learn/compose-volumes-and-tools
  - recipes/06-compose-volume-and-tools
prerequisites:
  - concepts/everything-is-a-file
usedInFlows:
  - {flow: compose-volumes-and-tools, step: 5}
honestLimits:
  - "Runners are deterministic in-process v0 (model/sha256/upper built-ins); the process runner is a later crate."
  - "ctl run is synchronous in v0 — the job is terminal when the write returns, so 'watch status while it runs' is a contract you exercise across callers, not within one."
  - "Job views are per-principal; the CLI presents the persisted ~/.wanix/dialer.key, so a job survives a remount by the same user — but the principal is exactly that key file, and a different key (user, machine) is a stranger."
canonicalCaveatFor: []
---

# Jobs Are Files

A call to slow or effectful work is reified as a job directory — arguments are files, invocation is a `ctl` write, the result is a file with a name, a lifecycle, and retention (ADR 0009, `docs/adrs/0009-job-protocol.md`).

Most agent runtimes invoke tools as ephemeral RPC: the call exists only as an entry in someone's transcript, vanishes with the process, and can never be safely retried after an unknown outcome. The job protocol is the opposite bet. A call is a *directory* the platform retains:

```text
spec.json  params.schema.json  health  usage  new
jobs/<id>/{in, params.json, ctl, out, err, status, result.json}
```

Read `new` and you get an opaque id. Write your input bytes to `in` (and optionally `params.json`), write `run` to `ctl`, then read `out` and `result.json`. This transcript ran verbatim against a `tool serve`d ToolFS mounted over the mesh ([Recipe 06](/recipes/06-compose-volume-and-tools)):

```text
/ $ cat < /n/upper/new
j77762dbf2540e718
/ $ echo raw protocol, no sugar > /n/upper/jobs/j77762dbf2540e718/in
/ $ cat < /n/upper/jobs/j77762dbf2540e718/status
{"state":"receiving",...,"inputBytes":23,"outputBytes":0}
/ $ echo run > /n/upper/jobs/j77762dbf2540e718/ctl
/ $ cat < /n/upper/jobs/j77762dbf2540e718/out
RAW PROTOCOL, NO SUGAR
/ $ cat < /n/upper/jobs/j77762dbf2540e718/result.json
{"state":"done","exitCode":0,"durationMs":0,"inputBytes":23,"outputBytes":23,"error":null,"retryable":true}
```

## What the reification buys

**Watchability.** `status` is a live snapshot — the transcript caught the job in `receiving` before `run` sealed it. A supervisor inspects an agent's calls from outside by listing `jobs/`, not by trusting a self-report.

**Auditability and retry.** Terminal jobs are retained on a TTL (`crates/wanix-tool/src/service.rs:95`), `result.json` carries a typed error taxonomy (`crates/wanix-job/src/taxonomy.rs`) with an explicit `retryable` verdict, and the job id is the idempotency key: after a crash you *read the job* instead of guessing whether the mutation happened.

**Sugar stays sugar.** The shell's `tool PATH` builtin (`crates/wanix-sh/src/tool.rs`) is exactly the file dance above plus error rendering — convenience over the protocol, never a bypass. Anything that can read and write files (a script, an agent, a remote peer) is already a complete job client.

**Confinement.** `ToolService::open_view(principal)` (`crates/wanix-tool/src/service.rs:81`) binds each caller — identified by the transport's verified peer key, never a payload field — to a private `jobs/` view. Foreign job ids read as `NotFound`.

Because a ToolFS is a plain `FileSystem`, all of this [imports across the mesh for free](/concepts/devices-import-for-free): the transcript above crossed a QUIC connection without the protocol knowing.

## Status / honest limits

- v0 runners are deterministic and in-process (`model`/`sha256`/`upper`); the process runner is a later crate, per `docs/toolfs.md` §"Runner Boundary".
- `ctl run` is synchronous in v0: terminal on return. The async lifecycle ADR 0009 describes (poll `status` mid-run) is the contract's shape, not yet an observable behavior of the built-ins.
- The "survives caller death" promise now crosses invocations: the CLI dials with the persisted `~/.wanix/dialer.key`, so a remount resumes the previous mount's jobs (`same_identity_redial_resumes_jobs`, `crates/wanix-cli/src/tool/serve/tests.rs`). The principal *is* that key file — protect it accordingly.
