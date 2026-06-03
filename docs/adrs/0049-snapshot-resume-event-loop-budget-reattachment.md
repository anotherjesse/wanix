# ADR 0049: Snapshot Resume Event Loop Budget Reattachment

## Status

Accepted

## Context

ADR 0042 added an explicit future-timer wait budget for fresh `wanix-rust qjs`
tasks, and ADR 0043 added an explicit fixed ready-IO turn budget. Persistent
`qjs-snapshot` and `qjs-resume` commands create or restore a
`QuickJsTaskRuntime` directly, so they previously only used the runtime's
default immediate post-eval drain.

That left persistent demos unable to prove the same bounded lifecycle behavior
as fresh `qjs` runs: a snapshot script could schedule a future timer but exit
before it fired, and a resume script could install a read handler but only use
the default single nonblocking turn.

## Decision

`wanix-rust qjs-snapshot` and `wanix-rust qjs-resume` accept
`--event-loop-ms N` and `--ready-io-turns N`. The CLI applies those budgets when
evaluating the script attached to the current `QuickJsTaskRuntime`.

The budgets are host lifecycle policy. They are selected by each snapshot or
resume invocation and are not serialized into QuickJS VM snapshot bytes.

## Consequences

Persistent QuickJS demos can now run bounded future timers before writing a VM
image, and restored tasks can run future timers or repeated ready-fd callbacks
under explicit native Wanix CLI policy.

This does not add a general scheduler, blocking readiness, streaming stdin,
signals, cancellation, open-ended interval lifecycle policy, serialized
resource-policy manifests, or a full Wanix task checkpoint format.
