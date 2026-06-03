# ADR 0006: Preflight Snapshot Metadata

## Status

Accepted

## Context

Persisted snapshots can contain a large WebAssembly linear memory image. Hosts
that store snapshots for multiple QuickJS wasm builds need to route, reject, or
size-check those bytes before paying to copy the full image into a `Snapshot`.
The byte envelope already contains the format version, ABI version, module hash,
memory length, and restore pointer fields needed for compatibility validation.

At the same time, guest pointers remain per-instance capabilities. Exposing the
saved stack, runtime, or context pointers as public metadata would make internal
restore details look like stable Rust API.

## Decision

Expose `SnapshotMetadata` and `Snapshot::metadata_from_bytes` for cheap
compatibility preflight. Metadata parsing validates the complete v1 restore
header, including saved guest pointer fields, but returns only route-friendly
fields: snapshot format version, QuickJS wasm ABI version, wasm module SHA-256,
and captured memory length.

Use the same metadata preflight inside `Snapshot::from_bytes_for_module` so
wrong-module and minimum-memory errors are reported before copying the embedded
WebAssembly memory image. Keep raw guest pointer fields private and omit them
from `Snapshot` debug output.

## Consequences

- Storage layers can inspect snapshot compatibility without copying large memory
  images.
- Module mismatch diagnostics can include both the snapshot and module SHA-256
  identities before restore is attempted.
- Metadata preflight remains a validation API, not a partial or lenient parser.
- Guest stack, runtime, and context pointers stay implementation details even
  though their presence is required by the byte format.
