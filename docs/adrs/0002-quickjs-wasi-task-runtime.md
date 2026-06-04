# ADR 0002: QuickJS/WASI Task Runtime Boundary

## Status

Accepted

## Context

QuickJS is the first serious Wanix task runtime outside Chrome. It runs in
Wasmtime as a WASI guest, but it must not become a second process model and it
must not inherit host WASI filesystem or fd semantics.

The durable boundary is the split between Wanix-owned process state and
QuickJS/Wasmtime engine mechanics. Wanix owns task identity, namespace, cwd,
argv/env, stdio, fd tables, service files, terminal attachments, exit state,
WASI syscall semantics, and host execution policy. The engine crate owns
runtime instantiation, guest memory decoding, fixture loading, snapshots, and
provider plumbing.

## Decision

Treat `qjs` as a real Wanix task driver:

- `#task/new/qjs` allocates task identity and service files.
- `cmd`, `env`, and `dir` are Wanix task metadata and flow into QuickJS through
  argv/env/cwd, not through JavaScript globals.
- fd 0/1/2 and later fds are Wanix task fds that may be backed by memory files,
  namespace files, host mounts, terminal devices, or service-file handoffs.
- fd binds capture shared open-file handles so service-file handoffs remain
  usable after the source task closes its fd.
- exit status is observable through Wanix task state.

`wanix-wasi` owns the Preview 1 syscall semantics used by Rust Wanix tasks. It
is backed by Wanix namespaces, task fds, explicit preopens, and Wanix filesystem
traits rather than by host WASI filesystem semantics. Its contract covers root
and cwd mapping, service paths such as `#task` and `#term`, fd operations,
rights projection, path and metadata operations, readiness, clocks/timers, and
deliberate unsupported errors for behavior that has no Wanix contract.

When a QuickJS task opens dynamic regular-file WASI fds, the adapter mirrors
the visible fd into the Wanix task fd table at the same number and removes it
when WASI closes the fd. Directory fds can stay WASI-internal until a Wanix task
fd contract needs them.

The checked-in QuickJS WASI fixture is a compatibility boundary for guest
modules and imports. Guest code should use `qjs:std`, `qjs:os`, `scriptArgs`,
stdio, environment, and service files. Engine-level read-only virtual files may
remain only as isolated fixture support; Wanix runtime paths must use live
Wanix-backed WASI providers.

Live Wanix-backed WASI providers are runtime host state attached through create
or restore options. They do not belong inside deterministic, cloneable
`QuickJsHostConfig` fixture policy.

QuickJS snapshots are VM images, not serialized Wanix task checkpoints. Restore
must explicitly reattach Wanix host state: namespaces, mounts, cwd, argv/env,
stdio, task fds, live WASI providers, clocks/random policy, terminal
attachments, execution limits, and exit observation. Open dynamic descriptors
block snapshot unless a future decision defines selected serializable virtual
fd state.

QuickJS timers, promise jobs, ready-IO handlers, interrupt callbacks, and heap
limits are bounded host execution policy. CLI and serve surfaces may expose
those knobs for deterministic demos and tests, but they are not a general
scheduler, signal system, cancellation model, or task checkpoint format.

## Consequences

JavaScript running through `qjs:std`, `qjs:os`, `scriptArgs`, stdio, and
`#task` service files reaches Wanix-owned task, filesystem, fd, and lifecycle
semantics without `globalThis.Wanix` helpers and without a read-only virtual
projection bridge.

Specific syscall coverage, fixture changes, and bounded-execution proofs belong
in `wanix-wasi`, `wanix-qjs`, `wanix-qjs-engine`, examples, tests, and commit
messages. Update this ADR only when the durable task runtime boundary changes.
