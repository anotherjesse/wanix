# ADR 0018: Expose Mixed Copied Function Calls

## Status

Accepted

## Context

ADR 0010 introduced copied scalar values, ADR 0016 introduced copied binary
values, and ADR 0017 allowed binary-capable host callbacks. Runtime-level
JavaScript function calls still had one asymmetry: Rust could receive a binary
function result, but binary inputs required a JavaScript wrapper or global
assignment first.

Hosts need to call JavaScript functions directly with copied scalar and binary
arguments and receive either copied scalar or binary results, without exposing
raw QuickJS handles or borrowed guest-memory pointers.

## Decision

Introduce `QuickJsCopiedValue` as the shared copied value wrapper for APIs that
can exchange either scalar or binary values:

- `QuickJsCopiedValue::Scalar(QuickJsValue)`
- `QuickJsCopiedValue::Binary(QuickJsBinaryValue)`

Keep `QuickJsCallbackValue` as a source-compatible alias for callback APIs.

Add `QuickJsRuntime::call_global_function_with_values(name, args)` for direct
function calls with copied scalar and binary arguments and copied scalar or
binary results. The function is read from `globalThis[name]` and called with
`this = undefined`, matching `call_global_function` and
`call_global_function_binary`.

The API creates temporary raw QuickJS values for the function, `this`, and all
arguments, calls the function, copies the result into `QuickJsCopiedValue`, and
frees all raw handles before returning. Binary inputs are copied into
QuickJS-owned `ArrayBuffer` or exact `Uint8Array` values; binary outputs are
copied into Rust-owned `Vec<u8>` values. The method requires both scalar and
binary helper exports and fails early when either value surface is unavailable.

No snapshot format change is needed. Values passed into or returned from the
call are ordinary QuickJS heap state while the call runs, and any persistent
state remains inside the existing linear-memory snapshot.

## Consequences

- Rust can now call JavaScript transforms with byte payloads directly instead
  of installing wrapper globals first.
- Existing scalar-only and binary-result call helpers remain available for
  narrower call sites.
- `QuickJsCallbackValue` remains valid for callback code, while
  `QuickJsCopiedValue` is the preferred non-callback name for scalar-or-binary
  copied values.
- ADR 0019 later adds copied typed-array views, and ADR 0021 later adds copied
  `DataView` values, to this mixed copied-value call path; objects, arrays,
  functions, promises, symbols, bigints, zero-copy views, and raw handle APIs
  remain future decisions.
