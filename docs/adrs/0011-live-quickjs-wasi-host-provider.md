# ADR 0011: Live QuickJS WASI Host Provider

## Status

Accepted

## Context

The relocated `wanix-qjs-engine` crate previously exposed deterministic
QuickJS host policy through `QuickJsHostConfig`: clock, random, timezone,
stdio capture, and immutable virtual files. That shape is useful because it is
cloneable, comparable, debug-redacted, and easy to reattach after snapshot
restore.

Wanix now needs QuickJS WASI imports to call Wanix-owned filesystem and fd
semantics instead of copying a read-only namespace projection into the engine.
The live provider is mutable host state: it owns fd allocation, descriptor
offsets, stdio attachments, preopens, namespace resolution, and rights. Putting
that state directly into `QuickJsHostConfig` would make deterministic policy
look like runtime resources and would create friction around equality, debug
output, clone behavior, and future snapshot/restore boundaries.

## Decision

Keep `QuickJsHostConfig` as deterministic host import policy.

Expose live WASI filesystem hooks through an engine-owned
`QuickJsWasiHost` trait. The trait uses Preview 1-shaped primitives and
metadata so `wanix-qjs-engine` remains responsible only for Wasmtime import
wiring and guest-memory copying.

Attach the live provider through `QuickJsCreateOptions::with_wasi_host(...)`.
The provider is runtime host state, not snapshot bytes and not Wanix policy
inside the engine crate.

Implement the Wanix adapter in `wanix-qjs` by wrapping `wanix_wasi::WasiCtx`.
That keeps Wanix task, namespace, fd, preopen, and stdio semantics in
`wanix-wasi`/`wanix-task`, while the engine remains a reusable QuickJS/Wasmtime
mechanics crate.

## Consequences

The current QuickJS WASM fixture can prove the first live WASI path through
stdio-adjacent imports (`fd_write`, `fd_fdstat_get`, `fd_seek`, and `fd_close`)
plus metadata imports in synthetic WAT tests. Broader path and read calls can
use the same trait as the fixture exposes more WASI surface.

The old read-only virtual filesystem remains a fallback for simple engine and
namespace demos. Wanix-backed task/config paths should prefer the live provider
and no longer need to flatten or reject additional Wanix preopens.

Snapshot restore still reattaches deterministic `QuickJsHostConfig` only. A
future cycle should add an explicit restore-time live-provider option when
snapshot/restore resumes Wanix-backed tasks.
