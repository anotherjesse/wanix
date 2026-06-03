# ADR 0003: Keep Linear Memory Capabilities Private

## Status

Accepted

## Context

The QuickJS WASI boundary exposes guest pointers for C strings, argument
vectors, and `JSValue*` handles. These values are capabilities into one live
WebAssembly instance and one live QuickJS runtime. They are not stable Rust
values, cannot be reused across snapshot/restore boundaries, and must usually be
released by calling back into the guest.

## Decision

Keep guest pointers and raw QuickJS value handles private to the runtime
implementation. Wrap `JSValue*` handles in a private `RawJsValue` type, reject
null handles at creation boundaries, and make public methods convert results
into ordinary Rust values or opaque snapshots before returning. Host-created
guest allocations and QuickJS handles must be cleaned up inside the method that
created them, including error paths. `RawJsValue` is a checked private token, not
a public ownership type; ownership is tracked by the method-local cleanup scope.

## Consequences

- Public APIs cannot accidentally leak raw linear-memory offsets or stale
  QuickJS handles.
- Snapshot compatibility remains a VM-image concern instead of becoming a
  promise that host-held JS handles survive restore.
- Cleanup-heavy methods need small helper functions so primary errors are not
  hidden by best-effort cleanup.
- Future host callback or external-resource APIs will need explicit handle
  registries instead of borrowing this raw pointer representation.
