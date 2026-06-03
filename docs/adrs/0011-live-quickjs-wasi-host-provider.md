# ADR 0011: Live QuickJS WASI Host Provider

## Status

Accepted

## Context

The relocated `wanix-qjs-engine` crate previously exposed deterministic
QuickJS host policy through `QuickJsHostConfig`: clock, random, timezone,
stdio capture, and immutable virtual files. That shape is useful because it is
cloneable, comparable, debug-redacted, and easy to reattach after snapshot
restore.

Wanix now needs QuickJS WASI imports to call Wanix-owned process, filesystem,
and fd semantics instead of copying a read-only namespace projection into the
engine. The live provider is mutable host state: it owns argv/env data, fd
allocation, descriptor offsets, stdio attachments, preopens, namespace
resolution, and rights. Putting that state directly into `QuickJsHostConfig`
would make deterministic policy look like runtime resources and would create
friction around equality, debug output, clone behavior, and future
snapshot/restore boundaries.

## Decision

Keep `QuickJsHostConfig` as deterministic host import policy.

Expose live WASI process and filesystem hooks through an engine-owned
`QuickJsWasiHost` trait. The trait uses Preview 1-shaped primitives and
metadata so `wanix-qjs-engine` remains responsible only for Wasmtime import
wiring and guest-memory copying.

Attach the live provider through `QuickJsCreateOptions::with_wasi_host(...)`
for fresh runtimes and `QuickJsRestoreOptions::with_wasi_host(...)` for
restored runtimes. The provider is runtime host state, not snapshot bytes and
not Wanix policy inside the engine crate.

Implement the Wanix adapter in `wanix-qjs` by wrapping `wanix_wasi::WasiCtx`.
That keeps Wanix task, argv/env, namespace, fd, preopen, and stdio semantics in
`wanix-wasi`/`wanix-task`, while the engine remains a reusable QuickJS/Wasmtime
mechanics crate.

## Consequences

The current QuickJS WASM fixture can prove live WASI plumbing through
QuickJS-NG `qjs:std` stdio plus process, metadata, path, directory, argv, and
env imports in synthetic WAT tests. `proc_exit` is treated as live process
state: the engine forwards it to the provider and then traps to honor the
non-returning WASI import shape, while Wanix decides how that exit request
updates task metadata. After the provider accepts `proc_exit`, the engine marks
the runtime as process-exited and skips ordinary QuickJS teardown when the
Wasmtime instance is dropped; a WASI process exit is terminal host state rather
than a normal JavaScript exception to clean up, resume, or snapshot. Broader
calls can use the same trait as the fixture exposes more WASI surface.

The old read-only virtual filesystem remains engine-only fixture support.
Wanix-backed task/config paths use the live provider and no longer flatten,
copy, or reject additional Wanix preopens for a virtual projection bridge.

Snapshot restore reattaches deterministic `QuickJsHostConfig` and live WASI
providers only through explicit restore options. The provider remains outside
the snapshot image, so Wanix task metadata and host resources can be restored
by Wanix policy instead of being serialized by the engine crate.
