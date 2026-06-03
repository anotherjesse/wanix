# ADR 0010: Own Copied Scalar JS Values

## Status

Accepted

## Context

The runtime previously exposed typed helpers such as `eval_number` and
`eval_string`, plus callback-specific `QuickJsHostValue` values. That was enough
for narrow examples, but Rust hosts also need to read globals, set globals, and
call JavaScript functions without flattening everything through ad hoc strings
or exposing raw QuickJS handles.

Raw handles are still per-runtime capabilities tied to cleanup obligations and
guest memory, so exposing them as a public API would require a larger ownership
design. A copied scalar API gives useful host control now while keeping that
trust boundary intact.

## Decision

Expose `QuickJsValue` as the public owned scalar value enum:

- `Undefined`
- `Null`
- `Bool(bool)`
- `Number(f64)`
- `String(String)`

The enum is `non_exhaustive` so future copied scalar variants can be added
without promising that callers have seen the whole JavaScript value space.
Runtime-affine handle APIs should use a separate type. `QuickJsHostValue`
remains as a compatibility alias for callback APIs, and new code should prefer
`QuickJsValue`.

`Number(f64)` uses ordinary Rust `f64` equality for the derived `PartialEq`:
`NaN` is not equal to itself and `+0.0` compares equal to `-0.0`.

Add scalar runtime APIs:

- `QuickJsRuntime::eval_value(code)`
- `QuickJsRuntime::get_global_value(name)`
- `QuickJsRuntime::set_global_value(name, value)`
- `QuickJsRuntime::call_global_function(name, args)`

These methods copy strings across the guest boundary and reject objects, arrays,
functions, promises, symbols, bigints, and other handle-backed values with a
clear error. `call_global_function` reads `globalThis[name]` and calls it with
`this = undefined`, so method-style calls should use a future method API or a
JavaScript wrapper when receiver binding matters. The runtime-level scalar APIs
fail early when the compiled QuickJS WASM module lacks any helper export in the
bundled scalar capability.

This does not change the snapshot format or the required ABI-v1 preflight.
Scalar helper exports remain optional prototype capabilities: minimal ABI
modules can still be loaded, while scalar APIs report unsupported capability
errors at use sites.

## Consequences

- Rust hosts can exchange ordinary copied values with restored runtimes without
  designing raw handle ownership first.
- Callback APIs now accept and return `undefined`, `null`, booleans, numbers,
  and strings through the same enum.
- Snapshot semantics stay VM-image based: scalar globals and functions survive
  as QuickJS state, while Rust closures and other host resources remain
  restore-time policy.
- Future handle APIs should be designed separately around explicit ownership,
  runtime affinity, and cleanup behavior.
