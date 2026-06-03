# ADR 0021: Expose Copied DataView JS Values

## Status

Accepted

## Context

ADR 0016 introduced copied `ArrayBuffer` and exact `Uint8Array` values. ADR
0019 extended that surface to typed-array wrappers while preserving only wrapper
kind and visible bytes. The remaining binary wrapper gap is JavaScript
`DataView`, which is commonly used to read and write mixed-width binary payloads.

Hosts need to exchange `DataView` values without raw QuickJS handles, borrowed
guest-memory pointers, mutable global constructor lookup, or property access that
guest JavaScript can replace or make effectful. A `DataView` can also expose a
window into a larger backing `ArrayBuffer`, so copying the full backing buffer
would leak bytes outside the JavaScript-visible view.

## Decision

Extend `QuickJsBinaryValue` with `DataView(Vec<u8>)` and a
`QuickJsBinaryValue::data_view(bytes)` constructor. Rust copies only the
`DataView` visible byte range. When Rust creates a `DataView`, the bytes are
copied into a new QuickJS-owned `ArrayBuffer` and wrapped by a fresh `DataView`
over the whole copied range.

The vendored QuickJS-ng API gains two C helpers:

- `JS_NewDataViewCopy(ctx, buf, len)` creates a `DataView` from copied bytes
  without looking up `globalThis.DataView`.
- `JS_GetDataViewBuffer(ctx, obj, offset_out, length_out)` checks the exact
  QuickJS `DataView` class and reports its backing `ArrayBuffer`, byte offset,
  and current visible length without reading JS properties.

The WASM interface exposes those helpers as optional exports:

- `qjs_new_data_view`
- `qjs_is_data_view`
- `qjs_get_data_view_buffer`

Runtime and binary-capable host callback code bind the exports optionally.
Existing `ArrayBuffer`, `Uint8Array`, and typed-array behavior continues to work
with older modules. Actual `DataView` use requires the new helper exports at the
use site and reports the missing helper when unavailable.

No snapshot format change is needed. Live `DataView` objects remain ordinary
QuickJS heap state inside the wasm linear-memory snapshot.

## Consequences

- Rust can evaluate, store, pass to JavaScript functions, receive from
  JavaScript functions, and exchange through binary-capable callbacks copied
  `DataView` byte windows.
- Scalar callbacks remain source-compatible and reject `DataView` arguments
  before copying, just like other binary values.
- The API preserves `DataView` wrapper identity and visible bytes, but
  intentionally does not preserve backing-buffer sharing, prototypes, detached
  state, resizable-buffer behavior, or typed element decoding.
- The C boundary avoids guest-observable constructor and property access, and
  relies on QuickJS's own detached and out-of-bounds `DataView` checks before
  copying.
- Structured object graphs, borrowed/zero-copy views, and raw handle APIs remain
  future decisions.
