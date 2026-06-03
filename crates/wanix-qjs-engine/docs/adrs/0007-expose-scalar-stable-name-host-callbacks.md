# ADR 0007: Expose Scalar Stable-Name Host Callbacks

## Status

Accepted

## Context

The QuickJS WASM fixture already supports host callbacks by storing a stable
callback name inside a QuickJS function object and dispatching calls through the
`env.host_call` import. That function object survives a linear-memory snapshot,
but the Rust closure behind the stable name is host state and cannot be
serialized safely into snapshot bytes.

The public Rust API must also preserve the earlier decision that raw QuickJS
handles and guest pointers stay private.

## Decision

Expose a narrow scalar callback API:

- `QuickJsRuntime::define_global_host_function(name, callback)` creates a
  JavaScript global function and registers the Rust callback for the live
  runtime.
- `QuickJsRuntime::register_host_callback(name, callback)` registers only the
  Rust callback implementation for a function object already present after
  restore.
- `QuickJsHostValue` is the only public callback value type for now, with
  `Undefined`, `Number(f64)`, and `String(String)` variants. It is
  non-exhaustive so future scalar and handle-backed variants can be added
  without breaking callers.
- Callback names are stable identifiers. Empty names and names containing NUL
  bytes are rejected.
- Callback closures are `FnMut(&[QuickJsHostValue]) ->
  anyhow::Result<QuickJsHostValue> + Send + 'static`.

Callback closures, captured Rust state, and callback registrations are not
serialized into snapshots. Restored runtimes must re-register callbacks by the
same stable names before calling snapshotted host function objects.

Callback support is an optional module capability, not a snapshot format or
QuickJS WASM ABI version bump in this prototype slice. The public callback
methods fail early if the compiled module does not export the helper functions
needed for callback creation, value conversion, and guest-visible errors.

Unsupported argument types, missing registrations, Rust callback errors, and
Rust callback panics are reported to JavaScript as thrown string errors.
Snapshot capture is rejected while a host callback is active.

ADR 0017 later adds opt-in binary-capable callback methods while keeping these
scalar callback methods source-compatible.

## Consequences

- Rust can now participate in guest execution without exposing raw QuickJS
  handles.
- Snapshots remain VM images plus compatibility metadata; they do not bundle
  Rust closures or captured host state.
- Restore behavior is explicit: callers decide which Rust implementation to bind
  to each stable callback name after restore.
- Objects, arrays, and richer handle-backed values remain future API work.
