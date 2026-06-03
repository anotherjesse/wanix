# ADR 0017: Expose Binary Host Callback Values

## Status

Accepted

## Context

ADR 0007 introduced stable-name Rust host callbacks with a scalar-only value
surface. ADR 0016 then introduced copied `ArrayBuffer` and exact `Uint8Array`
values for runtime eval, globals, and JavaScript function results.

Hosts now need JavaScript to call Rust with byte payloads and receive byte
payloads back without exposing raw `JSValue*` handles, guest pointers, or
borrowed ArrayBuffer views. The existing C callback trampoline already passes
heap-allocated `JSValue*` argument handles into `env.host_call` and accepts a
heap-allocated `JSValue*` return value, so no callback ABI or snapshot format
change is needed.

## Decision

Keep the existing scalar callback methods source-compatible:

- `QuickJsRuntime::define_global_host_function`
- `QuickJsRuntime::register_host_callback`

Add opt-in binary-capable callback methods:

- `QuickJsRuntime::define_global_host_function_with_binary_values`
- `QuickJsRuntime::register_host_callback_with_binary_values`

These methods use a sibling copied value type, now named `QuickJsCopiedValue`
for shared non-callback use and kept source-compatible through the
`QuickJsCallbackValue` alias:

- `QuickJsCopiedValue::Scalar(QuickJsValue)`
- `QuickJsCopiedValue::Binary(QuickJsBinaryValue)`

The nested shape keeps the scalar and binary public value contracts distinct.
Scalar callbacks continue to reject binary arguments with the same
guest-visible unsupported-argument error used for objects and functions.

Binary-capable callbacks support copied `ArrayBuffer` and exact `Uint8Array`
arguments and return values. Rust copies bytes from borrowed QuickJS data
pointers before invoking the callback closure and copies returned bytes into new
QuickJS-owned binary values before returning to the C trampoline. The host
import never frees callback argument handles because the C trampoline owns them.
It does free temporary guest allocations, including binary length slots and
return-value byte buffers.

Callback closures and registrations remain host state. Restored runtimes must
reattach binary-capable callbacks by the same stable names before JavaScript
calls snapshotted host function objects.

The binary-capable callback methods require both callback helper exports and
binary helper exports. Minimal ABI modules can still instantiate, scalar
callbacks do not require binary helpers, and binary-capable methods fail early
when the needed exports are unavailable.

ADR 0018 later reuses the same copied value wrapper for direct JavaScript
function calls with mixed scalar and binary arguments.

## Consequences

- JavaScript can now call Rust with copied byte payloads and Rust can return
  copied `ArrayBuffer` or `Uint8Array` values.
- Existing scalar callback callers keep their current signatures and rejection
  behavior.
- Raw QuickJS handles, callback argument ownership, and borrowed guest pointers
  remain private to the adapter/runtime boundary.
- Snapshot bytes remain unchanged; only restore-time host callback
  reattachment needs to choose the binary-capable API when the JavaScript host
  function expects byte values.
- ADR 0019 later adds copied typed-array views, and ADR 0021 later adds copied
  `DataView` values, to this binary-capable callback path; zero-copy views and
  handle-backed object values remain future API decisions.
