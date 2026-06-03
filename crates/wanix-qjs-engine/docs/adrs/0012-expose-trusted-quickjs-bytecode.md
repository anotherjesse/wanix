# ADR 0012: Expose Trusted QuickJS Bytecode

## Status

Accepted

## Context

The reference QuickJS/WASI project exposes `qjs_compile` and
`qjs_eval_bytecode`, allowing JavaScript source to be compiled to serialized
QuickJS bytecode and evaluated later. The Rust prototype already has copied
scalar values and module identity metadata, so it can expose the same capability
without making raw `JSValue` handles public.

QuickJS bytecode is not a stable, portable, or sandbox-verifiable interchange
format. It is tightly coupled to the exact QuickJS build and should be treated
as trusted input.

## Decision

Expose a trusted bytecode API:

- `QuickJsRuntime::compile_bytecode(code)`
- `QuickJsRuntime::compile_bytecode_with_options(code, filename, options)`
- `QuickJsRuntime::eval_bytecode_value(&bytecode)`
- `QuickJsRuntime::eval_bytecode_discard(&bytecode)`
- `QuickJsBytecode` stores copied bytecode bytes plus the producing module's
  SHA-256 identity.
- `QuickJsBytecodeCompileOptions` supports module compilation and source/debug
  stripping flags.

The QuickJS WASM adapter treats `qjs_compile` and `qjs_free_bytecode` as a
paired capability. `qjs_compile` returns a buffer allocated through QuickJS's
tracked allocator, so Rust must copy the bytes out and release that buffer with
`qjs_free_bytecode`, not generic `wasm_free`. Input buffers written by Rust
continue to use `wasm_free`.

`QuickJsBytecode::into_parts()` returns the module SHA-256 and serialized bytes
for persistence. `QuickJsBytecode::from_trusted_parts(wasm_sha256, bytes)`
intentionally names the trust boundary for reconstruction. It copies bytes and
binds them to the stored module identity, but it does not validate that the bytes
are safe or well-formed. Evaluation rejects bytecode whose stored module SHA-256
does not match the runtime's exact module SHA-256 before writing bytes into
guest memory.

The API returns copied scalar values or discards results. Raw QuickJS handles and
guest pointers remain private.

Bytecode support is an optional module capability in this prototype, not a
snapshot format or required QuickJS WASM ABI version bump. Public APIs fail
early when `qjs_compile`, `qjs_free_bytecode`, or `qjs_eval_bytecode` are
unavailable.

## Consequences

- Hosts can precompile trusted code, persist bytecode alongside their own
  metadata, transfer it between runtimes using the same module, and evaluate it
  after restore.
- Bytecode is exact-build-bound. Changing the QuickJS WASM fixture, compiler
  flags, C ABI, or QuickJS version should be treated as invalidating stored
  bytecode.
- The compile output buffer must stay on the QuickJS allocator path so memory
  limits and future memory usage diagnostics remain accurate.
- Debug output reports only byte length and module identity, because unstripped
  bytecode may contain source or debug metadata.
- Module bytecode still uses QuickJS module resolution, preserves source import
  specifiers, and therefore needs loader and normalizer policy to be installed
  whenever imports must be resolved after restore.
- A future public handle API should remain separate and continue to enforce
  runtime affinity and cleanup ownership explicitly.
