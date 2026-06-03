# ADR 0014: Select QuickJS Intrinsics At Create Time

## Status

Accepted

## Context

The reference QuickJS/WASI adapter exposes `qjs_init2`, which creates a
QuickJS context with a caller-provided bitmask of built-in JavaScript
intrinsics. This lets hosts omit features such as source `eval`, `Date`,
`Promise`, typed arrays, `Proxy`, and base64 helpers.

The Rust prototype already treats host imports as explicit Rust-side policy and
snapshots as initialized VM images. Intrinsic selection is different from host
imports: it changes the QuickJS context that is created before any JavaScript
runs. Restored snapshots should resume the context that already exists inside
the snapshot instead of reinitializing it with a new mask.

## Decision

Expose selective intrinsic creation through:

- `QuickJsIntrinsics`, a copied bitmask matching the reference adapter flags.
- `QuickJsCreateOptions`, which combines `QuickJsHostConfig` with optional
  creation-time intrinsic selection.
- `QuickJsModule::create_runtime_with_options(options)`.
- `QuickJsModule::create_runtime_with_intrinsics(intrinsics)`.
- Compatibility wrappers on `QuickJsRuntime` for callers still using the older
  runtime-owned construction style.

Default `create_runtime()` and `create_runtime_with_host_config()` continue to
call the required ABI-v1 `qjs_init` export. Passing explicit intrinsics opts in
to the optional `qjs_init2` export and fails early when the module does not
provide it. As with the other optional helper exports in this prototype, an
export that is present but mistyped is rejected during instantiation so adapter
ABI drift is caught early.

Restore APIs do not accept `QuickJsCreateOptions` or intrinsic masks. A
snapshot already contains an initialized QuickJS context, so restore continues
to accept only the host import configuration that should be reattached.

## Consequences

- Hosts can build smaller or more policy-constrained runtimes by selecting only
  the built-ins they need.
- Trusted bytecode becomes more useful because hosts can create eval-free
  runtimes and still execute bytecode compiled by a full runtime for the same
  module identity.
- Intrinsic selection is an optional module capability in this prototype, not a
  snapshot format or required QuickJS WASM ABI version bump.
- Base objects are always installed by the reference adapter and cannot be
  disabled through this bitmask.
- Unknown mask bits are preserved so compatible newer adapters can add flags
  without forcing an immediate Rust API change.
- Public APIs continue to keep raw QuickJS handles and guest pointers private.
