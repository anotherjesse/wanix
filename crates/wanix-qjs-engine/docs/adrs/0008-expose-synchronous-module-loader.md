# ADR 0008: Expose Synchronous Module Loader

## Status

Accepted

## Context

The QuickJS WASM fixture already contains a module-loader bridge:
`qjs_set_module_loader`, `env.host_module_normalize`, and
`env.host_module_load`. Rust previously linked those imports as stubs, so ES
module imports were not usable from the public runtime API.

Like host callbacks, module loading crosses the snapshot boundary. Loaded module
state lives in the QuickJS heap and can survive a linear-memory snapshot, but
the Rust closures that normalize names and load source code are host state.

## Decision

Expose a narrow synchronous module API:

- `QuickJsRuntime::set_module_loader(load)` installs a Rust source loader with
  pass-through module-name normalization.
- `QuickJsRuntime::set_module_loader_with_normalizer(normalize, load)` installs
  an explicit normalizer plus source loader.
- `QuickJsRuntime::eval_module_discard(code, filename)` evaluates source as an
  ES module without exposing raw QuickJS eval flags.

The loader and normalizer are `FnMut` closures returning owned Rust `String`
values. Guest C strings and module source buffers are copied into WebAssembly
memory before QuickJS receives them. Normalized module names and eval filenames
must not contain NUL bytes because QuickJS consumes them as C strings.

Module loader closures, captured Rust state, and registrations are not
serialized into snapshots. Restored runtimes must call a module-loader install
method again before future imports can load Rust-provided source. Already-loaded
module state remains part of the snapshotted QuickJS VM image.

Module loader support is an optional module capability in this prototype, not a
snapshot format or QuickJS WASM ABI version bump. Public loader APIs fail early
when the compiled module does not export `qjs_set_module_loader`. Loader and
normalizer errors currently surface through QuickJS's module machinery as
generic `ReferenceError`s, so Rust error text is not preserved yet.

## Consequences

- Rust hosts can evaluate module-mode JavaScript with static imports from
  virtual source maps or application-defined stores.
- The restore contract stays explicit: snapshots do not bundle filesystem,
  network, or closure state.
- Async loading, filesystem policy, import maps, and a general public `JSValue`
  handle remain future work.
- Snapshot capture is rejected while a module loader callback is active, matching
  the existing host-callback yield-point rule.
