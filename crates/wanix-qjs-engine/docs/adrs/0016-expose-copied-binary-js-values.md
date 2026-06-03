# ADR 0016: Expose Copied Binary JS Values

## Status

Accepted

## Context

Rust hosts need to pass byte payloads into QuickJS and read byte payloads back
out without exposing raw `JSValue*` handles or borrowed guest-memory pointers.
The reference adapter already has copied ArrayBuffer and Uint8Array helpers, but
the Rust API has intentionally kept public values scalar-only so far.

ArrayBuffer data pointers returned by QuickJS are borrowed pointers into the
wasm instance. They can become invalid after JavaScript execution or buffer
detachment, so Rust must copy the bytes immediately and never expose those
pointers to callers.

## Decision

Expose a sibling copied-value type, `QuickJsBinaryValue`, rather than expanding
the scalar `QuickJsValue` enum:

- `QuickJsBinaryValue::ArrayBuffer(Vec<u8>)`
- `QuickJsBinaryValue::Uint8Array(Vec<u8>)`

Add runtime APIs for the copied binary surface:

- `QuickJsRuntime::eval_binary_value(code)`
- `QuickJsRuntime::get_global_binary_value(name)`
- `QuickJsRuntime::set_global_binary_value(name, value)`
- `QuickJsRuntime::call_global_function_binary(name, scalar_args)`

The APIs bind the reference adapter's binary helpers as an optional bundled
capability. Minimal ABI modules can still instantiate, while binary APIs fail at
use sites when the needed exports are absent. Rust copies input bytes into
QuickJS-owned ArrayBuffer or Uint8Array values, copies output bytes into
Rust-owned `Vec<u8>` values, and frees all temporary guest buffers and raw
QuickJS handles before returning.

The first cycle supports exact `ArrayBuffer` and exact `Uint8Array` values.
Other typed arrays, `Uint8ClampedArray`, `DataView`, precise typed-array kind
metadata, binary host-callback arguments, and zero-copy borrowed views were left
as future API decisions in this first cycle. ADR 0017 later adds an opt-in
copied binary host-callback surface, ADR 0018 later adds direct JavaScript
function calls with copied binary arguments, and ADR 0019 later extends this
value API to copied typed-array views.

Returned data pointers are treated as borrowed and nullable-on-error. A null
data pointer is a failed adapter call even when the returned length slot is zero;
Rust takes the pending QuickJS exception before returning an error.

No snapshot format change is needed. Binary values survive snapshots as normal
QuickJS heap state inside the wasm linear-memory image.

## Consequences

- Rust hosts can round-trip byte payloads through globals, eval results, and,
  after ADR 0018, direct scalar-or-binary function calls without raw handle
  ownership.
- Public scalar APIs and scalar host callbacks keep their current scalar-only
  contract.
- QuickJS borrowed data pointers stay private and are copied before cleanup or
  any additional JavaScript execution.
- Fresh runtimes created with selective intrinsics must include typed arrays for
  JavaScript `ArrayBuffer` and `Uint8Array` constructors to exist.
- Broader BufferSource ergonomics can be added later without confusing copied
  bytes with runtime-affine handle APIs.
