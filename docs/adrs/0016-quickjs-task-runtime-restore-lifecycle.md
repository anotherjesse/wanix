# ADR 0016: QuickJS Task Runtime Restore Lifecycle

## Status

Accepted.

## Context

ADR 0003 defines QuickJS snapshots as VM images, not serialized Wanix task
records. ADR 0009 defines QuickJS as the execution engine inside a Wanix `qjs`
task, with Wanix owning task id, namespace, cwd/env/cmd, stdio/fds, and exit
state.

Wanix now needs a caller-facing lifecycle helper that can create a live
QuickJS runtime from a task, snapshot the VM image, restore that image, and
continue JavaScript execution while reattaching Wanix task resources outside
Chrome.

## Decision

Expose `QuickJsTaskRuntime` from `wanix-qjs` as the task-level owner for a live
QuickJS VM attached to a Wanix task. `QuickJsRunner::create_task_runtime`
creates a fresh runtime from a task. `QuickJsRunner::restore_task_runtime_from_bytes`
restores QuickJS VM bytes and reattaches host state from the supplied task.

The snapshot bytes remain QuickJS/Wasm memory only. On each create or restore,
`wanix-qjs` installs Wanix-backed WASI imports, task stdout/stderr callbacks,
the namespace module loader, `scriptArgs`, an interrupt handler, and a fresh
exit-state cell. `QuickJsTaskRuntime::finish` records the observed exit code on
the task, defaulting to `0` when JavaScript has not requested a status.

Guest memory and initialized guest process state, such as globals and QuickJS
libc environment data, may survive restore. Wanix host context exposed through
`scriptArgs`, `qjs:std`/`qjs:os`, task service files, stdio fds, namespace
files, and task exit state is restore-time state from the supplied task.

Snapshots must reject open dynamic WASI descriptors. The engine asks the live
`QuickJsWasiHost` provider for snapshot blockers, and the Wanix adapter reports
open dynamic fds from `wanix_wasi::WasiCtx`, including directory descriptors
that are not mirrored into the Wanix task fd table.

## Consequences

Wanix can prove a `qjs` task can run JavaScript, snapshot, restore, and continue
with preserved VM state while observing restore-time Wanix task identity,
namespace, stdio, service-file context, and exit handling.

This is not yet a full persisted task snapshot format. Future persisted task
snapshots still need explicit metadata for task identity, namespace bindings,
restart policy, and any selected serializable fd state.
