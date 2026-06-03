# ADR 0013: Expose Memory Usage And GC Controls

## Status

Accepted

## Context

The Rust runtime already exposes QuickJS heap limits, stack limits, and
interrupt handlers. The checked-in QuickJS WASM fixture also exports explicit
GC and memory usage helpers from the reference adapter:

- `qjs_run_gc`
- `qjs_set_gc_threshold`
- `qjs_get_gc_threshold`
- `qjs_compute_memory_usage`

Hosts that manage long-lived or restored runtimes need to observe QuickJS heap
pressure, tune automatic GC, and run GC before lifecycle points such as
snapshotting. This should not expose QuickJS handles, guest pointers, or C
struct layout directly.

## Decision

Expose copied GC and memory diagnostics:

- `QuickJsRuntime::run_gc()`
- `QuickJsRuntime::set_gc_threshold(bytes)`
- `QuickJsRuntime::disable_automatic_gc()`
- `QuickJsRuntime::gc_threshold()`
- `QuickJsRuntime::memory_usage()`
- `QuickJsMemoryUsage`, a copied Rust struct mirroring QuickJS-NG's
  `JSMemoryUsage` field order as signed 64-bit diagnostic counters.

`memory_usage()` allocates a private guest scratch buffer, asks QuickJS to write
the 26 `JSMemoryUsage` fields, copies the bytes into Rust, and frees the scratch
buffer before returning. Guest pointers and raw C structs stay private.

QuickJS uses `size_t::MAX` as the disabled automatic-GC threshold sentinel. On
the wasm32 ABI, Rust passes and reports that value as `u32::MAX`.

GC/memory support is an optional module capability in this prototype, not a
snapshot format or required QuickJS WASM ABI version bump. Public APIs fail
early when the corresponding helper exports are unavailable.

## Consequences

- Hosts can observe QuickJS heap accounting, read the effective malloc limit,
  tune automatic GC, and run GC explicitly from Rust.
- The counters are QuickJS diagnostics, not Rust-native ownership or allocation
  guarantees. They are intentionally signed to preserve the C ABI shape.
- Explicit GC can collect unreachable QuickJS heap objects, but it does not
  imply that WebAssembly linear memory shrinks.
- Numeric GC thresholds live in QuickJS runtime memory and may be carried by
  snapshots. Hosts with required policy should still set thresholds explicitly
  after create and restore.
- Future fixture changes to `JSMemoryUsage` should update the Rust field mapping
  and tests together.
