# ADR 0015: Harden C Adapter Allocation-Failure Boundaries

## Status

Accepted

## Context

The QuickJS/WASI C adapter is the only layer that can directly move owned
`JSValue`s between QuickJS and WebAssembly heap pointers. Most Rust APIs keep
raw handles private, but the adapter still has to behave correctly when its own
wrapper allocations fail or when direct exported functions receive malformed
pointer/count inputs.

Two edge cases matter for the trust boundary:

- `jsvalue_to_heap` receives an owned `JSValue` and wraps it in a heap pointer
  for host-visible transfer. If the wrapper allocation fails, the adapter must
  consume the owned `JSValue` instead of leaking it.
- `qjs_call` receives an argument count and an argv pointer from the host ABI.
  It must reject invalid counts, null pointers, size overflows, and argument
  array allocation failure before copying argv entries.
- Multi-output helpers such as promise creation and promise rejection
  notifications must not publish partial null handles after a wrapper
  allocation failure.

## Decision

Keep raw `JSValue` ownership in the C adapter and harden allocation failures in
place:

- `jsvalue_to_heap` frees the owned value and throws QuickJS out-of-memory when
  the wrapper pointer cannot be allocated.
- `qjs_call` rejects null function/receiver pointers, negative counts, null
  argv for nonzero argc, argument-count size overflow, null argument pointers,
  and temporary argument-array allocation failure.
- Promise rejection notifications skip host dispatch unless both duplicated
  values were wrapped successfully.
- `qjs_new_promise` validates output pointers, rejects aliased output slots,
  initializes outputs to null, and publishes resolve/reject handles only after
  all three promise-related handles are wrapped successfully.
- The TypeScript host layer treats null promise rejection pointers and partial
  `qjs_new_promise` outputs as failed adapter calls, freeing any non-null
  handles before returning or throwing.
- The public TypeScript `newPromise()` wrapper preflights its output-slot
  allocations, zero-initializes them before crossing into the adapter, and uses
  a fresh memory view to read them after the adapter may have grown memory.
- `qjs_free_value(NULL)` is a no-op.
- These adapter failures surface as QuickJS exceptions when a wrapper can be
  allocated, preserving the existing Rust/TypeScript exception path.

The Rust public API still avoids exposing raw QuickJS handles or guest pointers.

## Consequences

- Wrapper allocation failure no longer leaks non-immediate QuickJS values.
- Malformed direct `qjs_call` inputs become JavaScript exceptions rather than C
  null dereferences or unchecked heap writes.
- Promise helper failures no longer leave the host with zero-pointer handles
  that would later be dereferenced.
- The host promise APIs now fail closed if a malformed or future adapter build
  returns partial promise outputs.
- Normal Rust host-call paths are unchanged because Rust already preflights
  argument vectors and writes well-formed argv buffers.
- Allocation failure that also prevents wrapping the thrown exception can still
  appear to the host as a null return, but the owned `JSValue` has been consumed
  before returning.
