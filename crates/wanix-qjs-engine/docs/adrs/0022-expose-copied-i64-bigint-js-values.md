# ADR 0022: Expose Copied I64 BigInt JS Values

## Status

Accepted

## Context

ADR 0010 introduced copied scalar values for `undefined`, `null`, booleans,
numbers, and strings while keeping raw QuickJS handles private. JavaScript
`BigInt` is the remaining primitive-like value that Rust hosts commonly need
when JavaScript state uses integer identifiers, offsets, or counters that should
not be rounded through `Number`.

The existing reference adapter already exposed `qjs_new_big_int64` and
`qjs_get_big_int64`, but QuickJS's public `JS_ToBigInt64` helper returns values
modulo 2^64. That conversion is useful for typed-array and DataView semantics,
but it would be surprising for a copied Rust value API because `2n ** 63n`
would silently become `i64::MIN`.

## Decision

Add `QuickJsValue::BigIntI64(i64)` to the copied scalar surface. The variant
name is explicit because the Rust API copies only exact signed 64-bit BigInts,
not arbitrary-precision JavaScript BigInts.

Runtime scalar APIs, scalar host callbacks, binary-capable host callbacks, and
mixed copied function calls all support the new variant:

- `QuickJsRuntime::eval_value`
- `QuickJsRuntime::get_global_value`
- `QuickJsRuntime::set_global_value`
- `QuickJsRuntime::call_global_function`
- `QuickJsRuntime::call_global_function_with_values`
- `QuickJsRuntime::define_global_host_function`
- `QuickJsRuntime::define_global_host_function_with_binary_values`

The adapter keeps the existing optional WASM export names:

- `qjs_is_big_int`
- `qjs_new_big_int64`
- `qjs_get_big_int64`

`qjs_get_big_int64` now rejects non-BigInt values, null output pointers, and
values outside the signed 64-bit range. It still uses `JS_ToBigInt64` for the
initial conversion, but creates a signed-64-bit BigInt from the result and
compares it with the original using QuickJS strict equality before returning
the low/high words. Oversized values throw a RangeError instead of wrapping.

The BigInt helper exports remain optional. Older scalar-capable modules still
support older scalar values; creating or reading `BigIntI64` values reports the
missing helper at the use site when the relevant export is unavailable.

No snapshot format change is needed. BigInts stored in JavaScript state remain
ordinary QuickJS heap values inside the wasm linear-memory snapshot.

## Consequences

- Rust can evaluate, store, pass to JavaScript, receive from JavaScript, and
  exchange through host callbacks exact signed 64-bit BigInts.
- Out-of-range BigInts fail explicitly instead of silently truncating, wrapping,
  or rounding through `Number`.
- The public API remains a copied-value API; arbitrary-precision BigInts,
  boxed BigInt objects, raw handles, and structured object graphs remain future
  decisions.
- Existing `QuickJsValue` pattern matches outside this crate remain protected by
  `#[non_exhaustive]`; inside the crate, scalar conversion paths must handle the
  new variant explicitly.
