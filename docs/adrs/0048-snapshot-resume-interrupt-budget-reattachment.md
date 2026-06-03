# ADR 0048: Snapshot Resume Interrupt Budget Reattachment

## Status

Accepted

## Context

ADR 0045 added an explicit QuickJS interrupt-poll budget for fresh
`wanix-rust qjs` runs. ADR 0047 later applied the same host-policy framing to
heap limits for persistent `qjs-snapshot` and `qjs-resume` commands.

Persistent snapshot commands create or restore a `QuickJsTaskRuntime` directly
instead of going through the ordinary `QuickJsTaskDriver::start` path. That
means CPU-bound resume scripts could still run without the explicit
interrupt-poll budget available to fresh `qjs` tasks unless the persistent
composition layer reattached the policy itself.

## Decision

`wanix-rust qjs-snapshot` and `wanix-rust qjs-resume` accept
`--interrupt-after N`. The CLI installs the budget on the attached
`QuickJsTaskRuntime` before evaluating the snapshot or resume script.

The interrupt handler remains host state. It preserves Wanix process-exit
interruption and adds the poll budget in the same callback. The budget is not
serialized into the snapshot file; each snapshot or resume invocation decides
whether to apply it.

If the budget is exhausted during `qjs-snapshot`, no snapshot file is written.
If the budget is exhausted during `qjs-resume`, the restored task fails with
exit status `1` while preserving stdout/stderr emitted before the failure.

## Consequences

Persistent QuickJS demos can now prove both major explicit runtime bounds:
CPU-bound loops are stopped by reattached interrupt policy, and
allocation-heavy code is stopped by reattached heap policy.

This does not add wall-clock cancellation, Wasmtime fuel, signals, preemptive
scheduling, serialized resource-policy manifests, or a full Wanix task
checkpoint format.
