# ADR 0009: Expose Runtime Limits And Interruption

## Status

Accepted

## Context

Hosts running user-authored JavaScript need a way to bound execution. The
QuickJS WASM fixture already exports helpers for interrupt dispatch, heap
allocation limits, and stack limits, but Rust previously linked
`env.host_interrupt` as a no-op and did not expose those runtime controls.

This is both an externally visible capability and a trust-boundary improvement:
without it, a guest can run an infinite loop or allocate aggressively until the
embedding process intervenes.

## Decision

Expose a narrow runtime-limits API:

- `QuickJsRuntime::set_interrupt_handler(handler)` installs a Rust closure that
  QuickJS polls during execution. Returning `true` interrupts the current
  JavaScript execution.
- `QuickJsRuntime::clear_interrupt_handler()` disables interrupt dispatch.
- `QuickJsRuntime::set_memory_limit(bytes)` and `clear_memory_limit()` set or
  remove the QuickJS heap allocation limit. `0` means unlimited.
- `QuickJsRuntime::set_max_stack_size(bytes)` and `clear_max_stack_size()` set
  or remove the QuickJS stack limit. `0` means unlimited.

The byte counts are `u32` because these fixture exports take wasm32 `size_t`
parameters.

Interrupt closures and captured Rust state are not serialized into snapshots.
Restored runtimes must install a new interrupt handler when cancellation is
required. If an interrupt handler panics, the host import treats the panic as an
interrupt request so unwinding does not cross the Wasm boundary. Rust's panic
hook still runs.

QuickJS stores numeric memory and stack limits in runtime memory, so a snapshot
may carry their current values. Hosts with a required policy should still set
limits explicitly after create and after restore.

Runtime-limit support is an optional module capability in this prototype, not a
snapshot format or required QuickJS WASM ABI version bump. Public limit APIs
fail early when the compiled module does not export the needed helper.

## Consequences

- Hosts can stop runaway JavaScript and cap QuickJS heap allocations through the
  Rust API.
- Restore policy stays explicit for Rust closures, while numeric QuickJS runtime
  fields remain part of the VM image.
- This does not replace outer Wasmtime controls such as fuel or epoch
  interruption, and deep native/Wasm stack exhaustion can still surface as a
  lower-level trap before QuickJS turns it into a JavaScript exception.
- Memory-usage reporting and GC threshold control are covered separately by ADR
  0013. Creation-time QuickJS intrinsic selection is covered separately by ADR
  0014.
