# ADR 0004: Use A Versioned Snapshot Byte Format

## Status

Accepted

## Context

`Snapshot` started as an in-memory Rust envelope around a WebAssembly memory
image and the QuickJS pointer metadata needed for restore. That proves the VM
image technique, but it does not let callers persist a snapshot and resume it in
another process. The byte format is now a compatibility contract, so it needs
explicit versioning and strict parsing.

## Decision

Expose `Snapshot::try_to_bytes` as the fallible serializer, keep
`Snapshot::to_bytes` as its compatibility convenience wrapper, and expose
`Snapshot::from_bytes` and `Snapshot::from_bytes_for_module` for parsing. Encode
snapshots as a canonical little-endian binary envelope with:

- 8-byte magic: `RWQSNAP\0`.
- `u32` snapshot format version.
- `u32` QuickJS WASM ABI version.
- `u32` header length.
- `u64` total byte length.
- `u64` WebAssembly memory length.
- `u32` stack pointer.
- `u32` runtime pointer.
- `u32` context pointer.
- 32-byte QuickJS WASM SHA-256.
- Raw WebAssembly memory bytes.

The parser rejects invalid magic, unsupported versions, unexpected header
lengths, total-length mismatches, memory-length mismatches, trailing data,
non-page-aligned memory, invalid pointers, and unsupported ABI metadata.
Compression is not part of the format; it can be applied as an outer storage
layer.

## Consequences

- Snapshots can now be persisted and restored outside the original Rust process.
- `from_bytes_for_module` gives callers an ergonomic way to parse and enforce
  ADR 0002's exact-module binding in one step.
- `QuickJsModule::restore_runtime_from_bytes` is the preferred one-call path for
  parsing persisted bytes, enforcing module identity, and restoring a runtime.
- Future format changes must bump the snapshot format version or use a larger
  supported header length.
- Extension metadata and host resource reattachment are still future format/API
  work.
