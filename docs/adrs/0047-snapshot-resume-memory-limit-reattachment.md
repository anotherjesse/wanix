# ADR 0047: Snapshot Resume Memory Limit Reattachment

## Status

Accepted

## Context

ADR 0046 added an explicit QuickJS heap memory limit for `wanix-rust qjs`.
Persistent snapshot commands use a different path: `qjs-snapshot` creates a
`QuickJsTaskRuntime`, writes QuickJS VM bytes, and `qjs-resume` restores those
bytes into fresh Wanix task host resources.

Snapshot bytes remain QuickJS VM images. They must not become an implicit policy
container for Wanix task identity, fds, namespace bindings, host mounts, or
runtime resource policy. A memory limit needed after restore must therefore be
chosen by the restoring composition layer and applied to the restored runtime.

## Decision

`wanix-rust qjs-snapshot` and `wanix-rust qjs-resume` accept
`--memory-limit-bytes N`. The CLI applies the limit to the attached
`QuickJsTaskRuntime` before evaluating the snapshot or resume script.

If the limit is exhausted during `qjs-snapshot`, no snapshot file is written.
If the limit is exhausted during `qjs-resume`, the restored task fails with exit
status `1` while preserving stdout/stderr emitted before the failure.

The limit is not serialized into the snapshot file. Each snapshot or resume
invocation decides whether to apply the policy.

## Consequences

Persistent QuickJS demos can now prove both sides of the host-policy boundary:
VM state persists across invocations, while heap limits are selected and
reattached by the native Wanix composition layer.

This does not add a general serialized resource-policy format, stack limits, or
a full task checkpoint format. ADR 0048 later adds interrupt-budget
reattachment for snapshot/resume using the same host-policy boundary.
