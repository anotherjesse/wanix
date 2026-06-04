# wanix-qjs-engine

Workspace-local engine crate for a snapshot-able and resume-able JavaScript
runtime built around QuickJS-NG compiled to a WASI Preview 1 WebAssembly
reactor.

This crate was relocated from the `rust-wasi-quickjs` prototype into Wanix so
the QuickJS/WASI import boundary can evolve alongside Wanix-owned task,
namespace, and fd semantics. It includes a small Wasmtime host split across
`src/runtime.rs`, `src/host.rs`, and their child modules. The engine proves the
core restore mechanism against the checked-in fixture at `fixtures/quickjs.wasm`:

1. Instantiate `quickjs.wasm`.
2. Initialize QuickJS and evaluate JavaScript.
3. Snapshot the entire WebAssembly linear memory, `__stack_pointer`, and the
   QuickJS runtime/context pointers.
4. Instantiate a fresh module.
5. Grow and overwrite its linear memory from the snapshot.
6. Restore the runtime/context pointers and stack pointer.
7. Continue evaluating JavaScript from the restored state.

The tests validate ordinary global state, pending Promise resume behavior,
promise rejection tracking, snapshot/module compatibility checks, typed eval
conversion guards, snapshot byte-format properties, and host import
transfer/capture boundaries, copied scalar and binary values, host callback
reattachment, and synchronous ES module loading.

See [docs/architecture.md](docs/architecture.md) for the crate-local boundary
summary. The old prototype ADRs were consolidated there; the root Wanix
[ADR index](../../AGENTS.md#adr-index) is now the authoritative decision set.

## Quick Start

Run the executable snapshot/restore example against the checked-in fixture from
the Wanix workspace root:

```sh
cargo run --locked --package wanix-qjs-engine --example snapshot_restore
```

Additional examples show focused resume behavior. Run these from
`crates/wanix-qjs-engine`, or add `--package wanix-qjs-engine` when running from
the Wanix workspace root:

```sh
cargo run --locked --example bytecode_restore
cargo run --locked --example host_policy_restore
cargo run --locked --example host_callback_restore
cargo run --locked --example intrinsics_bytecode
cargo run --locked --example memory_gc_restore
cargo run --locked --example module_loader_restore
cargo run --locked --example pending_promise_resume
cargo run --locked --example promise_rejection_restore
cargo run --locked --example runtime_limits_restore
cargo run --locked --example scalar_values_restore
```

`bytecode_restore` compiles trusted QuickJS bytecode, snapshots VM state, and
evaluates the bytecode after restore. `host_policy_restore` preflights snapshot
bytes, then restores with a different Rust-side host clock and timezone policy.
`host_callback_restore` keeps a JavaScript host function object across restore
while reattaching the Rust callback by stable name. `intrinsics_bytecode` creates
an eval-free runtime with selected built-ins, executes trusted bytecode, then
restores the runtime. `memory_gc_restore` reports QuickJS heap usage, runs
explicit GC, snapshots, restores, and reapplies a GC threshold.
`module_loader_restore` imports ES module source from a Rust map, snapshots the
loaded module state, and reattaches the Rust loader for future imports after
restore. `pending_promise_resume` snapshots an unresolved JavaScript Promise,
restores it, resolves it from Rust, and drains the resumed job queue.
`promise_rejection_restore` reattaches rejection tracking after restore and
observes a post-restore unhandled rejection. `runtime_limits_restore` restores a
VM image, reapplies runtime limits, and stops a runaway loop with an interrupt
handler.
`scalar_values_restore` reads, writes, and calls JavaScript with copied scalar
values after restore.

The preferred lifecycle keeps the compiled module in charge of runtime creation
and byte restore:

```rust
use rust_wasi_quickjs::{QuickJsHostConfig, QuickJsModule};
use wasmtime::Engine;

let engine = Engine::default();
let module = QuickJsModule::from_file(&engine, "fixtures/quickjs.wasm")?;

let create_config = QuickJsHostConfig::new().with_clock_time_ns(1_700_000_000_000_000_000);
let mut runtime = module.create_runtime_with_host_config(create_config)?;
runtime.eval_discard("globalThis.counter = 41; queueMicrotask(() => globalThis.counter += 1);")?;
let jobs = runtime.execute_pending_jobs_with_limit(8)?;
assert_eq!(jobs, 1);
let turns = runtime.execute_immediate_event_loop_with_limit(8)?;
assert_eq!(turns, 0);
runtime.execute_ready_io_event_loop_once()?;
let _status = runtime.execute_event_loop_with_wait_budget(8, std::time::Duration::from_millis(10))?;

let bytes = runtime.snapshot()?.try_to_bytes()?;
let restore_config = QuickJsHostConfig::new().with_clock_time_ns(1_800_000_000_000_000_000);
let mut restored = module.restore_runtime_from_bytes_with_host_config(
    &bytes,
    restore_config,
)?;

assert_eq!(restored.eval_number("counter")?, 42.0);
assert_eq!(restored.eval_number("Date.now()")?, 1_800_000_000_000.0);
```

Callers that do not need a custom Wasmtime engine can use
`QuickJsModule::from_file_with_default_engine(...)` or
`QuickJsModule::from_bytes_with_default_engine(...)`; the compiled module then
owns the engine used for runtime creation and restore.

Snapshots contain the WebAssembly VM image and compatibility metadata. Host
configuration is supplied again on restore, so clocks, randomness, stdio
capture, timezone, engine-only read-only virtual fixture files, host callback
registries, module loader closures, promise rejection handlers, and interrupt
closures remain explicit Rust-side policy.
Use `Snapshot::metadata_from_bytes(&bytes)` when routing persisted snapshots by
module hash or memory size without copying the embedded WebAssembly memory image;
it still validates the full restore header, including saved guest pointer fields.

### Selective intrinsics

Use `QuickJsIntrinsics` and `QuickJsCreateOptions` when a fresh runtime should
start with only selected JavaScript built-ins:

```rust
use rust_wasi_quickjs::{QuickJsCreateOptions, QuickJsIntrinsics};

let options = QuickJsCreateOptions::new()
    .with_intrinsics(QuickJsIntrinsics::EVAL | QuickJsIntrinsics::JSON);
let mut runtime = module.create_runtime_with_options(options)?;

assert_eq!(runtime.eval_string("typeof JSON")?, "object");
assert_eq!(runtime.eval_string("typeof Promise")?, "undefined");
```

Base objects such as `Object`, `Array`, `String`, `Number`, and `Error` are
always installed by the reference adapter. Passing explicit intrinsics opts into
the optional `qjs_init2` export; default runtime creation still uses the
required ABI-v1 `qjs_init` export. Restore APIs do not accept intrinsic masks
because snapshots already contain an initialized QuickJS context. If `EVAL` is
omitted, source `eval_*` APIs and source bytecode compilation fail by design,
but trusted bytecode compiled by a full runtime can still be evaluated by the
restricted runtime.

### Copied scalar values

Use `QuickJsValue` when Rust needs to exchange ordinary copied values with
JavaScript without exposing raw QuickJS handles:

```rust
use rust_wasi_quickjs::QuickJsValue;

runtime.set_global_value("offset", QuickJsValue::Number(40.0))?;
runtime.eval_discard("globalThis.addOffset = value => offset + value")?;

let answer = runtime.call_global_function(
    "addOffset",
    &[QuickJsValue::Number(2.0)],
)?;
assert_eq!(answer, QuickJsValue::Number(42.0));
```

The copied scalar surface supports `undefined`, `null`, booleans, numbers,
strings, and exact signed 64-bit BigInts for `eval_value`, global get/set,
scalar function calls, and host callbacks. `call_global_function` invokes an
extracted global function with `this = undefined`. Objects, arrays, functions,
promises, symbols, boxed BigInt objects, and arbitrary-precision BigInts remain
private QuickJS handles until a future ownership-safe handle API is designed.
Runtime-level scalar value support is a bundled optional capability: APIs fail
early when the module lacks the needed helper exports.

### Copied binary values

Use `QuickJsBinaryValue` when Rust needs to exchange byte payloads with
JavaScript without exposing raw QuickJS handles or borrowed guest pointers:

```rust
use rust_wasi_quickjs::{QuickJsBinaryValue, QuickJsTypedArrayKind};

runtime.set_global_binary_value(
    "payload",
    QuickJsBinaryValue::uint8_array(&[1, 2, 3, 4])?,
)?;
runtime.eval_discard("payload[1] = 99")?;

assert_eq!(
    runtime.get_global_binary_value("payload")?,
    QuickJsBinaryValue::Uint8Array(vec![1, 99, 3, 4])
);

runtime.set_global_binary_value(
    "words",
    QuickJsBinaryValue::typed_array(QuickJsTypedArrayKind::Uint16, &[1, 0, 2, 0])?,
)?;
assert_eq!(runtime.eval_string("words.constructor.name")?, "Uint16Array");

runtime.set_global_binary_value(
    "view",
    QuickJsBinaryValue::data_view(&[3, 1, 4, 1])?,
)?;
assert_eq!(
    runtime.eval_string("view instanceof DataView ? 'yes' : 'no'")?,
    "yes"
);
```

The copied binary surface supports `ArrayBuffer`, exact `Uint8Array`, and other
typed-array wrappers, plus `DataView`, for `eval_binary_value`, global get/set,
direct function calls, and binary-capable host callbacks. Typed arrays are
copied as their visible raw byte range plus an exact `QuickJsTypedArrayKind`;
DataViews are copied as their visible raw byte range without preserving the
backing buffer identity. Rust does not borrow guest memory or decode numeric
elements. Zero-copy views remain future API work. Runtime-level binary value
support is a bundled optional capability: APIs fail early when the module lacks
the needed helper exports for the requested value.

Use `QuickJsCopiedValue` when a JavaScript function call needs mixed copied
scalar and binary arguments or may return either copied kind:

```rust
use rust_wasi_quickjs::{QuickJsBinaryValue, QuickJsCopiedValue, QuickJsValue};

runtime.eval_discard(
    "globalThis.sumBytes = bytes => bytes.reduce((sum, byte) => sum + byte, 0)",
)?;

assert_eq!(
    runtime.call_global_function_with_values(
        "sumBytes",
        &[QuickJsBinaryValue::uint8_array(&[10, 20, 12])?.into()],
    )?,
    QuickJsCopiedValue::Scalar(QuickJsValue::Number(42.0))
);
```

### Trusted bytecode

Hosts can compile trusted source to QuickJS bytecode and evaluate it later,
including after restoring a snapshot:

```rust
use rust_wasi_quickjs::{QuickJsBytecodeCompileOptions, QuickJsValue};

let bytecode = runtime.compile_bytecode_with_options(
    "globalThis.ran = true; 40 + 2",
    "trusted.js",
    QuickJsBytecodeCompileOptions::new().strip_source().strip_debug(),
)?;

let bytes = runtime.snapshot()?.try_to_bytes()?;
let mut restored = module.restore_runtime_from_bytes(&bytes)?;
assert_eq!(
    restored.eval_bytecode_value(&bytecode)?,
    QuickJsValue::Number(42.0)
);
```

QuickJS bytecode is trusted-only and exact-build-bound. `QuickJsBytecode` stores
the producing module SHA-256, and evaluation rejects mismatched module identity
before writing bytes into guest memory. Persist the SHA-256 alongside
`bytecode.bytes()`, then reconstruct with `QuickJsBytecode::from_trusted_parts`;
this copies bytes and binds them to the stored module identity, but it does not
validate untrusted bytecode. Debug output intentionally hides serialized bytes,
which may contain source/debug metadata unless stripped. Module bytecode still
uses QuickJS module resolution and preserves source import specifiers, so hosts
must reinstall loader and normalizer policy when imports need to be resolved
after restore.

### Host callbacks

Rust callbacks can be exposed as JavaScript globals through stable names. The
JavaScript function object is snapshotted; the Rust closure is host state and is
reattached after restore:

```rust
use rust_wasi_quickjs::QuickJsHostValue;

runtime.define_global_host_function("hostAdd", |args| match args {
    [QuickJsHostValue::Number(left), QuickJsHostValue::Number(right)] => {
        Ok(QuickJsHostValue::Number(left + right))
    }
    _ => anyhow::bail!("hostAdd expects two numbers"),
})?;

let bytes = runtime.snapshot()?.try_to_bytes()?;
let mut restored = module.restore_runtime_from_bytes(&bytes)?;
restored.register_host_callback("hostAdd", |args| match args {
    [QuickJsHostValue::Number(left), QuickJsHostValue::Number(right)] => {
        Ok(QuickJsHostValue::Number(left + right + 100.0))
    }
    _ => anyhow::bail!("hostAdd expects two numbers"),
})?;
```

The callback value surface is intentionally scalar: `undefined`, `null`,
booleans, numbers, and strings. Other JavaScript argument types are reported as
guest-visible errors until a richer handle API exists. Callback support is an
optional fixture capability: ABI-v1 modules still preflight
without callback helper exports, and callback APIs fail early when those helper
exports are unavailable.

When a callback needs copied bytes, use the opt-in binary-capable methods and
the callback-facing `QuickJsCallbackValue` alias:

```rust
use rust_wasi_quickjs::{QuickJsBinaryValue, QuickJsCallbackValue};

runtime.define_global_host_function_with_binary_values("hostBytes", |args| {
    match args {
        [QuickJsCallbackValue::Binary(QuickJsBinaryValue::Uint8Array(bytes))] => {
            Ok(QuickJsBinaryValue::array_buffer(bytes)?.into())
        }
        _ => anyhow::bail!("hostBytes expects one Uint8Array"),
    }
})?;
```

These callbacks support scalar values plus copied `ArrayBuffer`, exact
`Uint8Array`, typed-array, and `DataView` arguments and return values. Scalar
callback methods remain source-compatible and continue to reject binary
arguments.
`QuickJsCopiedValue` is the preferred name for the same scalar-or-binary copied
wrapper outside callback APIs.

### ES modules

Rust hosts can provide a synchronous ES module loader, with an optional
normalizer for relative or virtual module names:

```rust
runtime.set_module_loader_with_normalizer(
    |_base_name, specifier| match specifier {
        "./math.js" => Ok("math.js".to_string()),
        other => Ok(other.to_string()),
    },
    |name| match name {
        "math.js" => Ok("export const answer = 42;".to_string()),
        _ => anyhow::bail!("missing module {name}"),
    },
)?;

runtime.eval_module_discard(
    r#"import { answer } from "./math.js"; globalThis.answer = answer;"#,
    "main.js",
)?;
```

Module loader closures are host state, like callback registries and host
configuration. Loaded module state is part of the snapshotted QuickJS VM image,
but restored runtimes must install a loader again before future imports can load
Rust-provided source. Loader support is an optional fixture capability; public
loader APIs fail early when `qjs_set_module_loader` is not
available. Rust loader and normalizer error details are currently collapsed to
QuickJS module `ReferenceError`s such as `could not load module`.

### Runtime limits

Hosts can bound QuickJS execution with heap allocation limits, stack limits, and
an interrupt handler:

```rust
runtime.set_memory_limit(8 * 1024 * 1024)?;
runtime.set_max_stack_size(1024 * 1024)?;

let mut polls = 0usize;
runtime.set_interrupt_handler(move || {
    polls += 1;
    polls > 1
})?;
```

Returning `true` from the interrupt handler stops the current JavaScript
execution with a QuickJS `interrupted` exception. Interrupt closures are Rust
host state and must be installed again after restore. Numeric memory and stack
limits live in QuickJS runtime memory and may be present in a snapshot, but
hosts should set them explicitly after create and restore whenever policy
matters.

### Memory and GC diagnostics

Hosts can inspect copied QuickJS memory counters, tune automatic GC, and run GC
explicitly:

```rust
runtime.set_gc_threshold(512 * 1024)?;
let before = runtime.memory_usage()?;
runtime.run_gc()?;
let after = runtime.memory_usage()?;
println!("QuickJS heap: {} -> {} bytes", before.memory_used_size, after.memory_used_size);
```

`QuickJsMemoryUsage` mirrors the reference adapter's flattened `JSMemoryUsage`
field order as signed 64-bit diagnostic counters. The counters describe QuickJS
heap accounting; they do not imply that WebAssembly linear memory shrinks after
GC. GC threshold values live in QuickJS runtime memory and may be present in a
snapshot, but hosts should set required GC policy explicitly after create and
restore. Use `disable_automatic_gc()` when the host wants to turn off automatic
GC entirely. Memory/GC support is an optional fixture capability; public APIs
fail early when the corresponding helper exports are
unavailable.

### Promise rejection tracking

Hosts can install a diagnostic callback for unhandled promise rejections and for
later handler attachment notifications:

```rust
runtime.set_promise_rejection_handler(|event| {
    eprintln!(
        "promise rejection: reason={} handled={}",
        event.reason(),
        event.is_handled(),
    );
})?;
```

The event contains a copied reason string and an `is_handled` flag. Raw QuickJS
promise and reason handles stay private and are freed by the host import.
Reasons that cannot be stringified are reported with a lossy fallback string.
Rejection handlers are Rust host state and must be installed again after restore.
Tracking support is an optional fixture capability; public APIs
fail early when `qjs_set_promise_rejection_handler` is not available, and event
delivery relies on the standard required value/string/free exports.

### Capturing stdio

By default, WASI stdout/stderr writes inherit the process streams. Enable capture
when the host needs to inspect output, and use byte limits when guest output must
stay within an explicit retention budget:

```rust
let config = QuickJsHostConfig::new()
    .with_limited_stdout_capture(64 * 1024)
    .with_limited_stderr_capture(64 * 1024);

let mut runtime = module.create_runtime_with_host_config(config)?;
runtime.eval_discard("globalThis.ready = true")?;
let stdout = runtime.take_captured_stdout();
```

`take_captured_stdout()` and `take_captured_stderr()` clear the retained buffers,
so long-running hosts can drain output between evaluations and stay under their
configured per-stream limits. Host config is supplied again on restore; snapshot
bytes do not serialize stdio capture settings or retained output. The checked-in
QuickJS fixture initializes `qjs:std` and `qjs:os`, so guest code can import
`qjs:std`, write through WASI stdout/stderr, and call `qjs:os` filesystem
helpers such as `lstat`, `readlink`, and `symlink` through live WASI providers.
It also exposes `qjs:os.truncate` and `qjs:os.ftruncate` for live
`fd_filestat_set_size` providers.
The fixture still does not install `console`; higher-level adapters such as
`wanix-qjs` may install their own console shim.

### Engine-only read-only virtual files

Standalone engine tests and small fixtures can attach immutable files at
absolute guest paths without exposing host paths or mutable filesystem
authority:

```rust
let config = QuickJsHostConfig::new()
    .with_read_only_virtual_file("/app/config.json", br#"{"answer":42}"#)?;

let mut runtime = module.create_runtime_with_host_config(config)?;
```

When files are configured, the WASI shim exposes a single root preopen at `/`
and allows `path_open` only for normalized relative paths beneath that root.
Guest paths containing absolute prefixes, empty components, `.`, `..`, NUL
bytes, or backslashes are rejected before lookup, and configured absolute paths
are capped at 4096 bytes. Files support read, seek, tell, fdstat, and filestat
rights only; file metadata reports type and byte length, and parent paths
implied by configured files report as non-enumerable directories. Directory
listing, mutation, symlinks, and host path mounts are intentionally outside this
engine-owned virtual-file surface.

Virtual filesystem configuration is supplied again on restore just like the
clock, stdio, callback, and module-loader policies. Snapshot bytes do not
serialize file contents or open descriptor state, and `snapshot()` fails while a
virtual file descriptor is open.

Wanix runtime paths must not depend on these read-only virtual files for
namespace behavior. They are engine fixture support only. Mutable files,
directory listing, service paths, host mounts, stdio/fds, argv/env, process
exit, and timestamp mutation flow through live `QuickJsWasiHost` providers
attached with `QuickJsCreateOptions::with_wasi_host` or
`QuickJsRestoreOptions::with_wasi_host`; higher-level Wanix crates own the
actual process, namespace, and fd policy.

## Reference Build

The Wanix workspace does not vendor the Vercel reference source or build tree.
The checked-in reference WASM binary can be rebuilt from an external
`vercel-labs/quickjs-wasi` checkout plus the libc fixture changes summarized in
the root [QuickJS fixture ADR](../../docs/adrs/0012-quickjs-libc-std-fixture.md)
and this crate's [architecture notes](docs/architecture.md):

```sh
git clone https://github.com/vercel-labs/quickjs-wasi /tmp/quickjs-wasi
cd /tmp/quickjs-wasi
git checkout f26cf71
make setup
# Apply the Wanix fixture source/build changes before rebuilding.
make quickjs.wasm
cp quickjs.wasm /path/to/wanix/crates/wanix-qjs-engine/fixtures/quickjs.wasm
```

The Rust tests use `fixtures/quickjs.wasm` by default. Override that path with:

```sh
QUICKJS_WASM=/path/to/quickjs.wasm cargo test
```

## Checks

Run the repository gate before committing changes:

```sh
cargo fmt --package wanix-9p --package wanix-cli --package wanix-fs --package wanix-protocol --package wanix-qjs --package wanix-qjs-engine --package wanix-task --package wanix-term --package wanix-vfs --package wanix-wasi --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

The crate-local `scripts/check.sh` is retained for focused engine work, but the
Wanix root gate above is the required cycle check.

Run the dependency policy check separately when dependency policy
changes:

```sh
cargo install cargo-deny --version 0.19.8 --locked
scripts/advisory-check.sh
```

It runs `cargo-deny` against the locked dependency graph with all features
enabled, checking RustSec advisories, crate sources, and dependency bans for
non-dev duplicate versions and wildcard requirements. CI runs this as a
separate job so dependency policy findings do not blur the normal build/test
gate.

## Current Status

Working:

- Wasmtime host for the main QuickJS WASI reactor.
- Minimal `env.*` host imports.
- Minimal deterministic `wasi_snapshot_preview1` shim.
- `QuickJsModule` identity wrapper with SHA-256 module binding and module-owned
  runtime create/restore helpers.
- Opaque `Snapshot` capture and validated restore.
- Versioned snapshot byte serialization/deserialization.
- Explicit `QuickJsHostConfig` for deterministic clock, random, timezone, and
  optional stdio capture imports with per-stream byte limits.
- Optional live `QuickJsWasiHost` providers attached through
  `QuickJsCreateOptions` and `QuickJsRestoreOptions` for runtime-owned Preview
  1 fd/filesystem imports.
- Live provider forwarding for argv/env, process exit, stdio/fd reads and
  writes, path open/stat/create/remove/rename, directory listing, append fdflag
  mutation, explicit path/fd timestamp mutation, fd size mutation, and
  `poll_oneoff` clock plus live fd read/write readiness events.
- Read-only virtual WASI files attached through `QuickJsHostConfig` as
  engine-only fixture support, exposed under a normalized root preopen without
  host path mounts.
- Scalar Rust host callbacks exposed as JavaScript globals and reattached after
  restore by stable name when the loaded module exports the callback helpers.
- Opt-in binary-capable Rust host callbacks for copied `ArrayBuffer`, exact
  `Uint8Array`, typed-array, and `DataView` arguments and return values.
- Synchronous Rust ES module loader with optional name normalization and
  restore-time reattachment for future imports.
- Runtime memory limits, stack limits, and Rust interrupt handlers for
  cancelling runaway JavaScript execution.
- QuickJS memory usage diagnostics, explicit GC, and automatic GC threshold
  control.
- Promise rejection tracking with restore-time handler reattachment.
- Trusted QuickJS bytecode compilation/evaluation with exact module binding.
- Copied `ArrayBuffer`, exact `Uint8Array`, typed-array, and `DataView` values
  for eval, globals, and direct function calls with scalar-or-binary copied
  arguments and results.
- Eval helpers for numbers and strings.
- Safe public helper for calling a restored global function with a string.
- Pending job queue draining, including a bounded drain helper, with a restore
  test for pending promises.

Not implemented yet:

- Snapshot compression/framing and storage integrations beyond the canonical byte envelope.
- Native WASM extension dynamic linking and extension metadata restore.
- Engine-owned mutable virtual files, symlinks, or host path mounts. Use a live
  `QuickJsWasiHost` provider and higher-level Wanix crates for those semantics.
- General async event-loop policy beyond bounded pending-job drains, bounded
  future timer waits, timer-only sleeps, and immediately-ready live fd poll
  events.

See [docs/architecture.md](docs/architecture.md) for the current boundary
summary and [docs/investigation.md](docs/investigation.md) for the historical
snapshot investigation notes.
