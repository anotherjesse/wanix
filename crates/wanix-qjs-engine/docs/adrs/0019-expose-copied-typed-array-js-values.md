# ADR 0019: Expose Copied Typed-Array JS Values

## Status

Accepted

## Context

ADR 0016 introduced copied `ArrayBuffer` and exact `Uint8Array` values. ADR
0017 and ADR 0018 reused that copied-binary surface for host callbacks and
mixed scalar-or-binary function calls. The remaining binary gap was JavaScript
typed arrays such as `Int16Array`, `Uint16Array`, `Float32Array`,
`BigInt64Array`, and `Uint8ClampedArray`.

Hosts need to exchange those values without raw QuickJS handles or borrowed
guest-memory pointers. A typed array can be a view into a larger backing
`ArrayBuffer`, so copying the underlying buffer would expose bytes outside the
JavaScript-visible view and make subarray results surprising.

## Decision

Extend `QuickJsBinaryValue` with typed-array support:

- keep `QuickJsBinaryValue::ArrayBuffer(Vec<u8>)`
- keep `QuickJsBinaryValue::Uint8Array(Vec<u8>)` for source compatibility
- add `QuickJsBinaryValue::TypedArray { kind: QuickJsTypedArrayKind, bytes:
  Vec<u8> }`

`QuickJsTypedArrayKind` mirrors QuickJS's typed-array tags for
`Uint8ClampedArray`, integer arrays, float arrays, and BigInt arrays. Rust
copies only the visible raw byte range for a typed-array view and records the
exact JavaScript wrapper kind. Rust does not decode numeric elements into host
endian integer or float vectors.

The C adapter adds optional helpers to report a typed-array tag and create a
typed-array wrapper from copied bytes. Runtime and host-callback code bind those
helpers optionally. Existing `ArrayBuffer` and `Uint8Array` behavior keeps
working with older modules that have the ADR 0016 helpers; generic typed-array
values require the new helper exports at use sites.

The adapter also hardens `qjs_get_typed_array_buffer` against null direct ABI
inputs before writing metadata outputs.

No snapshot format change is needed. Typed arrays are ordinary QuickJS heap
state inside the wasm memory image while live.

## Consequences

- Rust can now evaluate, store, call with, and receive copied typed-array byte
  views without wrapper globals.
- Binary-capable host callbacks can accept and return typed arrays while scalar
  callbacks remain source-compatible and reject binary values before copying.
- The API preserves wrapper identity and visible bytes, but intentionally does
  not preserve shared backing-buffer identity, prototypes, detached/resizable
  state, or element-level numeric typing in Rust.
- ADR 0021 later adds copied `DataView` values. Structured object graphs,
  borrowed/zero-copy views, and native extension loading remain future API
  decisions.
