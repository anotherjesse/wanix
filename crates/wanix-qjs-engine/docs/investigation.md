# Investigation: Snapshot-able QuickJS/WASI Runtime in Rust

## Upstream Shape

The original prototype used a cloned reference project at
`reference/quickjs-wasi`, pinned to `vercel-labs/quickjs-wasi` commit
`f26cf71` with QuickJS-NG submodule commit `433941b`. That reference checkout is
not vendored in the Wanix workspace copy of this crate.

The upstream design is intentionally simple:

- Build QuickJS-NG plus a C interface as a `wasm32-wasip1` reactor.
- Export `memory`, `__stack_pointer`, malloc/free, the indirect function table,
  and a WASM-friendly `qjs_*` API.
- Keep one QuickJS runtime/context per WebAssembly instance.
- Snapshot raw linear memory and a few scalar values.
- Restore by creating a fresh instance, copying memory back, and restoring the
  saved pointers/globals.

Important upstream files:

- `reference/quickjs-wasi/Makefile`: reactor build flags, exported memory,
  exported `__stack_pointer`, and dynamic-linking exports.
- `reference/quickjs-wasi/c/interface.c`: QuickJS lifecycle, host-call
  trampoline, value API, and `qjs_set_runtime_and_context`.
- `reference/quickjs-wasi/src/index.ts`: TypeScript host implementation for
  `create`, `restore`, `snapshot`, and snapshot serialization.
- `reference/quickjs-wasi/src/extensions.ts`: extension dynamic-linking model.
- `reference/quickjs-wasi/test/snapshot.test.ts`: expected restore behavior,
  including pending promises and host callback re-registration.

## Core Insight

For this style of snapshotting, QuickJS must run inside WebAssembly rather than
as a native Rust FFI library. Native QuickJS pointers would be process addresses
and cannot be safely replayed into a different process or fresh engine instance.
Inside wasm32, those pointers are offsets into linear memory. If all engine state
is inside that linear memory, copying the memory image preserves the object graph,
atom table, closures, pending promise jobs, and QuickJS heap.

The snapshot is therefore not a semantic JavaScript serializer. It is a VM image.

## Rust Host Viability

Wasmtime exposes the primitives this design needs:

- Linker-defined imports for `env.*` and `wasi_snapshot_preview1.*`.
- Access to exported `memory`.
- Access to exported mutable globals such as `__stack_pointer`.
- Typed function calls into exported `qjs_*` functions.

The prototype in `src/` validates the Rust path with Wasmtime `45.0.0`. The
runtime now loads through `QuickJsModule`, records a SHA-256 hash for the wasm
bytes, and rejects snapshots whose hash, format version, ABI version, memory
shape, or core pointers do not match.

Tested behavior:

- `globalThis.counter = 42` survives snapshot/restore.
- A pending Promise and its `.then` job survive snapshot/restore; after restore,
  Rust retrieves the saved resolver function, calls it, drains the QuickJS job
  queue, and observes the completed state.
- Structurally invalid or wrong-module snapshots are rejected before any memory
  bytes are copied into a new instance.

## Restore Algorithm

Fresh create:

1. Compile/load `quickjs.wasm`.
2. Instantiate with Rust-provided imports.
3. Store exported memory in host state so WASI imports can read/write it.
4. Call `_initialize()`.
5. Call `qjs_init()`.

Snapshot:

1. Copy `memory.data()` into a byte vector.
2. Read `__stack_pointer`.
3. Call `qjs_get_runtime_ptr()`.
4. Call `qjs_get_context_ptr()`.
5. Store snapshot format version, ABI version, and wasm module hash alongside
   the memory image.

Restore:

1. Instantiate the same module with compatible imports.
2. Validate snapshot format version, ABI version, module hash, memory page
   alignment, non-empty memory, and core pointer ranges.
3. Do not call `_initialize()` or `qjs_init()`.
4. Grow memory to the snapshot size.
5. Copy snapshot bytes into linear memory at offset zero.
6. Call `qjs_set_runtime_and_context(runtime_ptr, context_ptr)`.
7. Read back the runtime pointer and context pointer to verify the guest
   reattached the saved VM handles.
8. Set `__stack_pointer`.
9. Read back `__stack_pointer` as a host global to verify the restored instance
   matches the snapshot metadata before returning.

The "same module" requirement matters. A snapshot should be tied to the exact
QuickJS/WASI build, including compiler/linker choices and extension layout.

## Rust Architecture Recommendation

Keep the crate split around the same boundaries as the reference project:

- `build/` or `quickjs-sys-wasm`: build QuickJS-NG and the C interface into
  `quickjs.wasm`.
- `runtime`: Wasmtime host, imports, exported function bindings, handles,
  snapshot/restore.
- `snapshot`: versioned binary format, compatibility metadata, compression hook.
- `host`: callback registry, module loader, interrupt handling, rejection
  handler, timezone/clock/random configuration.
- `extensions`: optional dylink loader for WASM shared libraries.

The Rust public API can look roughly like:

```rust
let engine = wasmtime::Engine::default();
let module = QuickJsModule::from_file(&engine, "quickjs.wasm")?;

let mut vm = module.create_runtime()?;
vm.eval_discard("globalThis.counter = 42")?;
let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;

let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
assert_eq!(restored.eval_number("counter")?, 42.0);
```

## Host Callback Plan

Upstream host callbacks are snapshot-friendly because the QuickJS C function
stores a name string in QuickJS heap state. After restore, the host registers a
new Rust callback under the same name.

The Rust prototype now implements the first scalar global callback slice with
`QuickJsHostValue`, `define_global_host_function`, and
`register_host_callback`. See ADR 0007 for the public contract.

Rust implementation path:

1. Add `HashMap<String, HostCallback>` to the Wasmtime store state or runtime.
2. Implement `env.host_call(name_ptr, name_len, this, argc, argv)` to decode the
   name and dispatch into the map.
3. Add `new_function(name, callback)` that stores the Rust callback and calls
   `qjs_new_host_function`.
4. Add `register_host_callback(name, callback)` for restore-time reattachment.

Host callback closures themselves are not in the snapshot. Applications must
re-register them after restore.

## Module Loader Plan

The Rust prototype now implements the first synchronous ES module loader slice
with `set_module_loader`, `set_module_loader_with_normalizer`, and
`eval_module_discard`. The loader is intentionally host-owned state: module
source and normalized names are copied into guest memory for each import, while
Rust closures and captured stores are not serialized into snapshot bytes.

Already-loaded module state survives snapshot/restore because it lives in the
QuickJS heap. Future imports after restore require the host to install a module
loader again. See ADR 0008 for the public contract and the deferred async,
filesystem, import-map, and richer value-handle work.

## Runtime Limits Plan

The Rust prototype now exposes QuickJS runtime memory limits, stack limits,
interrupt handlers, memory usage diagnostics, explicit GC, and automatic GC
threshold controls. Interrupt closures are Rust host state and must be installed
again after restore. Numeric memory, stack, and GC threshold controls are stored
by QuickJS in runtime memory, so snapshots may carry their current values, but
hosts should set required policies explicitly after create and restore. See ADR
0009 and ADR 0013 for the public contracts and the deferred intrinsics work.

## Promise Rejection Plan

The Rust prototype now exposes copied promise rejection notifications with
`set_promise_rejection_handler` and `clear_promise_rejection_handler`. The host
import stringifies the reason, frees QuickJS's duplicated promise/reason values,
uses a lossy fallback when reason stringification fails, and catches Rust handler
panics before returning to Wasm. Promise rejection handlers are restore-time Rust
host state; see ADR 0011 for the public contract.

## Bytecode Plan

The Rust prototype now exposes trusted QuickJS bytecode compilation and
evaluation. Bytecode is copied into Rust-owned memory, bound to the producing
module SHA-256, and rejected before evaluation if the runtime module identity
differs. This is not an untrusted or portable interchange format; see ADR 0012
for the public contract.

## Extension Plan

The reference project supports native WASM extensions through dynamic linking:

- Shared libraries import the main module's memory, table, stack pointer, and
  exported QuickJS symbols.
- Each extension receives `__memory_base` and `__table_base`.
- Restore must instantiate extensions with the same memory/table bases so
  function table indices match the restored QuickJS heap.

Rust can reproduce this with Wasmtime, but extension support should be a second
milestone after the core runtime:

1. Parse `dylink.0` custom sections.
2. Allocate extension static memory via main-module `malloc`.
3. Grow and populate the indirect function table.
4. Resolve `env`, `GOT.mem`, and `GOT.func` imports.
5. Store extension metadata in the snapshot.
6. On restore, instantiate extensions before copying snapshot memory, with the
   original bases, and skip extension init functions.

## Snapshot Format Plan

The current v1 snapshot byte format is a canonical little-endian envelope:

- Magic bytes: `RWQSNAP\0`.
- Snapshot format version.
- QuickJS WASM ABI version.
- Header length and total byte length.
- Raw WebAssembly memory length.
- Stack pointer.
- Runtime pointer.
- Context pointer.
- QuickJS WASM content hash.
- Raw WebAssembly memory bytes.

Future versions should add extension metadata through a format-version bump or a
larger supported header. Compression should remain an outer layer. Raw wasm
memory contains large zero regions and should compress well with zstd or gzip.

## Risks And Constraints

- Snapshots are VM images, not portable JS values. They require the same ABI and
  a compatible module build.
- Host resources are not captured. Files, sockets, timers, Rust futures, and
  callback closures need app-level restore policy.
- Module loader closures are not captured. Loaded module state can survive
  restore, but future imports need explicit loader reattachment.
- Interrupt closures are not captured. Numeric QuickJS memory and stack limits
  may survive as VM state, but required host policies should be applied after
  create and restore.
- Promise rejection handlers are not captured. Reattach them after restore when
  async diagnostics matter.
- Copied scalar values can cross the Rust/JavaScript boundary after create and
  restore, but raw QuickJS handles remain private runtime capabilities.
- Deterministic WASI time/randomness are configurable; broader host resources
  still need explicit restore policy.
- Native QuickJS FFI crates are useful for ordinary embedding, but they do not
  provide this memory-image portability.
- Extension function tables are separate from linear memory; they must be
  reconstructed on restore.
- Snapshotting while host callbacks, module loader closures, promise rejection
  handlers, or interrupt handlers are actively on the stack should be avoided;
  snapshot at well-defined yield points.

## Next Milestones

1. Promote the internal raw value helpers into a real ownership-safe
   `JSValueHandle` if user-facing handle APIs are needed.
2. Extend host import configuration for filesystem and broader WASI policy.
3. Move the QuickJS WASM build into this repo instead of relying on the reference
   directory.
4. Add extension dynamic linking only after the main runtime API is stable.
