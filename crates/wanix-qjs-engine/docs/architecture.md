# QuickJS Engine Architecture Notes

`wanix-qjs-engine` was relocated from the `rust-wasi-quickjs` prototype into
the Wanix workspace. The prototype carried one ADR per public API slice. In the
Wanix tree, the root ADR index is the durable decision set, and this file keeps
the engine-specific invariants in one place.

See the root ADRs for the Wanix-facing decisions:

- [Rust-native Wasmtime runtime](../../../docs/adrs/0001-rust-native-wasmtime-runtime.md)
- [QuickJS/WASI task runtime boundary](../../../docs/adrs/0002-quickjs-wasi-task-runtime.md)

## Engine Role

The engine crate owns the Wasmtime and QuickJS plumbing. It does not own Wanix
task identity, namespaces, cwd/env/cmd, service files, fd-table policy, WASI
semantics, or runtime lifecycle semantics. Those stay in `wanix-task`,
`wanix-vfs`, `wanix-wasi`, and `wanix-qjs`.

QuickJS runs as a WASI Preview 1 WebAssembly reactor. Native QuickJS pointers
are not portable across process or runtime boundaries; inside wasm32, the
pointers are offsets into WebAssembly linear memory. That is why snapshots are
VM images instead of JavaScript serialization.

## Snapshot Boundary

A snapshot contains:

- a versioned little-endian envelope;
- the QuickJS WASM ABI version;
- the exact QuickJS WASM SHA-256 module identity;
- the WebAssembly linear-memory image;
- the saved `__stack_pointer`;
- the saved QuickJS runtime and context pointers.

Restore validates the envelope and module identity before copying memory into a
fresh instance. The raw guest pointers remain implementation details; public
metadata APIs expose only route-friendly compatibility data such as format
version, ABI version, module hash, and memory length.

Snapshots do not serialize Rust host state. Clocks, random policy, stdio
capture, callback registries, module loaders, promise rejection handlers,
interrupt closures, live WASI providers, and open host resources must be
reattached at create or restore time.

## Host State

`QuickJsHostConfig` is deterministic host import configuration. It carries
cloneable policy such as clocks, random bytes, timezone, stdio capture limits,
and engine-only read-only virtual fixture files.

Live `QuickJsWasiHost` providers are runtime host state. They are attached
through `QuickJsCreateOptions::with_wasi_host` and
`QuickJsRestoreOptions::with_wasi_host` so hosts such as Wanix can own mutable
filesystem, process, namespace, fd, stdio, and service-file semantics without
putting live handles inside deterministic config.

Read-only virtual files in `QuickJsHostConfig` are retained only for standalone
engine tests and tiny immutable fixtures. Wanix runtime paths must use live
WASI providers instead.

## Public Value Boundary

The engine keeps raw QuickJS handles and guest pointers private. Public APIs
copy values across the boundary or use stable names that can be reattached:

- copied scalar values for ordinary primitive results and arguments;
- copied binary values for `ArrayBuffer`, exact `Uint8Array`, typed-array, and
  `DataView` byte windows;
- exact signed 64-bit BigInt values;
- trusted QuickJS bytecode bound to the producing module hash;
- stable-name host callbacks, with scalar and binary-capable entrypoints;
- synchronous module loader callbacks, reattached after restore;
- promise rejection callbacks, reattached after restore.

The API intentionally avoids public raw `JSValue*` handles, borrowed guest
views, native extension handles, or structured object graphs until those
ownership and restore contracts are designed explicitly.

## Execution Bounds

The engine exposes bounded execution controls around QuickJS:

- pending job draining with explicit limits;
- immediate event-loop turns and ready-IO turns;
- wait-budgeted event-loop execution;
- interrupt handlers;
- QuickJS memory and stack limits;
- memory and GC diagnostics.

Numeric QuickJS limits may live inside VM memory and therefore appear in a
snapshot, but required host policy should be applied after each create and
restore. Rust closures remain host state and must be reattached.

## Fixture And ABI Compatibility

The checked-in `fixtures/quickjs.wasm` is part of the compatibility boundary.
Changing the QuickJS C adapter, exported helper set, libc fixture behavior,
compiler flags, or module build can invalidate older snapshots and trusted
bytecode. Public APIs should fail early when an optional helper export is not
available.

Fixture rebuild details and per-helper API history belong in the crate README,
tests, examples, and git history rather than in a separate ADR per helper.
