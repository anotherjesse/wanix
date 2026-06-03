# ADR 0011: Expose Promise Rejection Tracking

## Status

Accepted

## Context

QuickJS can notify the host when a promise is rejected without a handler, and
again when a handler is attached later. The WASM fixture already exports
`qjs_set_promise_rejection_handler` and dispatches duplicated promise/reason
values through the `env.host_promise_rejection` import, but the Rust host import
was previously a no-op.

That no-op hid async diagnostics from embedders and skipped an explicit
ownership boundary: the C trampoline transfers duplicated `JSValue` pointers to
the host, and the host must free them before returning to QuickJS.

## Decision

Expose a diagnostic-only Rust API:

- `QuickJsRuntime::set_promise_rejection_handler(handler)` enables QuickJS's
  rejection tracker and installs a Rust callback.
- `QuickJsRuntime::clear_promise_rejection_handler()` disables the tracker and
  removes the Rust callback.
- `QuickJsPromiseRejection` carries a copied reason string and an `is_handled`
  flag.

The public event intentionally does not expose promise or reason handles. The
host import converts the reason to a string with QuickJS, frees the temporary C
string, frees both duplicated `JSValue` pointers with `qjs_free_value`, and then
returns control to QuickJS. When no Rust handler is registered, the import still
frees both `JSValue` pointers and skips stringification.

Reason stringification is diagnostic-only. If QuickJS cannot convert a reason to
a string, the event uses a lossy fallback string and still frees the duplicated
promise/reason values.

Promise rejection handlers are Rust host state, not snapshot bytes. Restored
runtimes must install a handler again when they need rejection diagnostics. If a
Rust handler panics, Rust's panic hook still runs, but the host import catches
the panic so unwinding does not cross the Wasm boundary.

Promise rejection tracking is an optional module capability in this prototype,
not a snapshot format or required QuickJS WASM ABI version bump. Public APIs fail
early when `qjs_set_promise_rejection_handler` is unavailable. Event delivery
also relies on the standard required value/string/free exports that are already
part of the runtime ABI.

## Consequences

- Hosts can observe unhandled and later-handled promise rejections after create
  and after restore.
- Raw QuickJS promise/reason handles remain private and are always consumed by
  the host import.
- Rejection tracking remains explicit restore-time policy, like host callbacks,
  module loaders, and interrupt closures.
- Richer rejection payload APIs can be added later only after an ownership-safe
  handle design exists.
