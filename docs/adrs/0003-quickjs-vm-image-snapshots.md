# ADR 0003: QuickJS VM Image Snapshot Boundary

## Status

Accepted

## Context

QuickJS can serialize VM state when running as the checked-in WASI fixture, but
Wanix task state is larger than guest memory. A task has identity, namespace
bindings, cwd, argv, env, stdio, file descriptors, limits, event-loop policy,
terminal attachments, exit state, and host resources that cannot be safely
stored as opaque QuickJS bytes.

Snapshots are still useful for proving persistent JavaScript state across host
processes. The durable decision is the boundary between guest VM memory and
Wanix host/task state.

## Decision

QuickJS snapshots are VM images, not serialized Wanix task checkpoints.

Snapshot bytes are tied to a compatible QuickJS/WASI fixture build. Creating or
restoring a runtime must attach host state explicitly:

- cwd, argv, env, namespace, mounts, stdio, and task fds;
- live Wanix-backed WASI providers;
- interrupt-poll budgets, heap limits, event-loop wait budgets, ready-IO turn
  budgets, and other host lifecycle policy; and
- exit-state observation and process-status plumbing.

Open dynamic descriptors block snapshot unless a future design defines selected
serializable virtual fd state. Host-backed files, terminal fds, and service fds
are resources to reattach, not memory objects to embed in the snapshot.

`QuickJsTaskRuntime` owns create and restore lifecycle for QuickJS task VMs.
Persistent CLI commands such as `qjs-snapshot` and `qjs-resume` are demos of the
boundary: they can configure before-snapshot and after-restore argv/env/mounts
separately, but they do not create a full task checkpoint format.

## Consequences

Wanix can persist guest JavaScript state without pretending host resources are
pure data. Restore remains explicit, reviewable, and safe at the trust boundary.

Future snapshot work should name which host resources are serializable and how
they are reattached. Adding lifecycle knobs to create/restore options does not
change the snapshot file format unless the VM image compatibility boundary
changes.

## Replaces

This ADR consolidates ADR 0016, ADR 0018, ADR 0024, and ADRs 0047 through 0049
into the durable snapshot and host-resource reattachment boundary.
