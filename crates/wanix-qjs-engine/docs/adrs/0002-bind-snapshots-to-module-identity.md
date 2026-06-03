# ADR 0002: Bind Snapshots To Module Identity

## Status

Accepted

## Context

The raw restore mechanism copies arbitrary bytes into a fresh WebAssembly
instance and reinstalls saved QuickJS pointers. Without validation, safe Rust
could restore bytes from a different QuickJS build, a truncated memory image, or
forged runtime/context pointers. That turns a useful VM-image technique into an
unsafe public boundary.

## Decision

Represent compiled QuickJS wasm as `QuickJsModule`, recording a SHA-256 hash of
the exact wasm bytes and validating that the module exports the required runtime
ABI before callers can create or restore a VM. Represent snapshots as opaque
`Snapshot` values with a format version, ABI version, module hash, memory bytes,
stack pointer, runtime pointer, and context pointer. Validate the snapshot
against the module before instantiating and restoring memory.

Prefer runtime lifecycle helpers on `QuickJsModule` for create and restore so
stores and linkers use the same Wasmtime engine that compiled the module.
Compatibility APIs that still accept an explicit `Engine` must reject engines
that did not compile the module before instantiating.

## Consequences

- Restore rejects wrong-module and structurally invalid snapshots before
  copying memory into a new instance.
- Module construction rejects wasm builds that are missing the required memory,
  stack pointer, or QuickJS runtime function exports before runtime creation.
- Snapshot compatibility is enforced in code instead of only being documented.
- The shortest create/restore path is module-owned, reducing the chance that
  callers combine a module with the wrong Wasmtime engine.
- Compatibility APIs fail intentionally when callers combine a module with the
  wrong Wasmtime engine, instead of relying on lower-level Wasmtime errors.
- The in-memory snapshot envelope is ready for a future binary serializer.
- Changing the QuickJS C interface, compiler/linker flags, or fixture wasm
  should be treated as an ABI-affecting change and may require incrementing the
  ABI version.
