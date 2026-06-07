---
title: Compiled-Artifact Cache
slug: concepts/compiled-artifact-cache
pageType: concept
oneLiner: A sha256-keyed on-disk cache of Wasmtime-serialized modules cuts qjs cold start ~1000x, and the cache directory is an fd-verified owner-private trust boundary.
audience: [developer]
tags: [runtime, performance, trust-boundary, shipped, caveat]
sourceRefs:
  - crates/wanix-module-cache/src/lib.rs:1-132
  - crates/wanix-module-cache/src/trust.rs:20-95
  - crates/wanix-module-cache/src/dir.rs:34-109
  - crates/wanix-qjs/src/bundled.rs:16-40
  - crates/wanix-wasm/src/cache.rs:16-37
  - docs/adrs/0002-quickjs-wasi-task-runtime.md:83-138
seeAlso:
  - concepts/wasmtime-as-substrate
  - concepts/qjs-task
  - concepts/compiled-wasm-task-driver
  - concepts/two-tiers-one-substrate
prerequisites:
  - concepts/wasmtime-as-substrate
usedInFlows: []
honestLimits:
  - The strong fd-based ownership check is Unix-only; non-Unix degrades to "is a directory" and leans on a per-user default location.
  - The cache verifies the leaf directory and the artifact file, not every ancestor directory; it assumes the default per-user cache root has owner-private ancestors.
  - The sha256 key authenticates the input wasm, not the cached bytes, so the cache is advisory and a hostile artifact is recompiled rather than trusted.
---

# Compiled-Artifact Cache

A sha256-keyed on-disk cache of Wasmtime-serialized modules cuts qjs cold start ~1000x, and the cache directory is an fd-verified owner-private trust boundary.

Cranelift-compiling the ~1.7 MiB bundled QuickJS module costs roughly 550 ms — close to 100% of a qjs task's cold start (`docs/adrs/0002-quickjs-wasi-task-runtime.md:85-88`). The first task you run pays that once; every later task that loads the same wasm build should not. Wanix solves this the way Wasmtime intends: serialize the compiled module to a host- and version-bound artifact, then `deserialize` it on warm starts in well under a millisecond. Because `deserialize` loads native code, the file it reads from is a trust boundary, and the whole point of this page is that Wanix treats it like one.

## Show: the ~550 ms -> ~0.5 ms win

Run a JavaScript task and the engine has to build the QuickJS module before your script executes:

```sh
cargo build --package wanix-cli
alias wanix-rust='./target/debug/wanix-rust'

# Cold: compiles the ~1.7 MiB QuickJS fixture (~550 ms, ~100% of cold start).
time wanix-rust qjs examples/qjs-demo.js

# Warm: deserializes the cached artifact (~0.5 ms, ~1000x faster, ~5x lower peak RSS).
time wanix-rust qjs examples/qjs-demo.js
```

The second run does no Cranelift work. `QuickJsModule::from_bytes_cached` looks for a cached artifact first and only compiles on a miss (`crates/wanix-qjs-engine/src/module/load.rs:88`). The compiled-`.wasm` runtime takes the identical path — `WasiRunner::from_bytes_cached` caches arbitrary `wasm32-wasi` command modules through the same primitive (`crates/wanix-wasm/src/runner.rs:51`), so re-running a non-trivial wasm guest stops re-paying its compile too. This is an accepted runtime decision, not a test-fixture trick (`docs/adrs/0002-quickjs-wasi-task-runtime.md:88-91`).

## Name: the cache key authenticates the input wasm

The cache is keyed by `sha256(wasm-bytes)`, rendered as a hex filename with a `.cwasm` suffix (`crates/wanix-module-cache/src/lib.rs:73-80`). The key authenticates the *input wasm*, so changing the fixture lands the artifact under a new key — no manual invalidation.

The cache is **advisory**: a missing, stale, unreadable, or untrusted artifact simply falls back to a fresh `Module::new` compile (`crates/wanix-module-cache/src/lib.rs:102-132`). Two recompile-on-mismatch behaviours fall out of this. First, Wasmtime embeds its own engine/version compatibility marker in the serialized bytes and rejects mismatches on `deserialize`, so a Wasmtime upgrade recompiles automatically — the key stays the same, the old artifact is detected as incompatible and replaced (`crates/wanix-module-cache/src/lib.rs:67-72`). Second, any `deserialize` error at all is treated as a cache miss, never a fatal error. `load_or_compile` returns an error only if the *fresh* compile fails.

## Name: the cache directory is a trust boundary

Here is the load-bearing fact. `Module::deserialize` is `unsafe` and equivalent to loading native code into your process, and the sha256 key authenticates the input wasm, *not* the cached `.cwasm` bytes on disk (`crates/wanix-module-cache/src/lib.rs:20-25`). A local attacker who could pre-seed `<sha256>.cwasm` in a world-writable cache directory would otherwise get arbitrary code execution in the victim's process.

So the cache refuses to read or write through a directory that is not private and owner-only. On creation the directory tree is made `0o700` (`crates/wanix-module-cache/src/trust.rs:110-121`). The default location is never a shared temp path: it resolves to the per-user cache root (`$XDG_CACHE_HOME` / `$HOME/.cache` / `%LOCALAPPDATA%`), and only as a last resort to a UID-scoped subdirectory of the system temp dir (`crates/wanix-module-cache/src/dir.rs:34-69`). An untrusted or hostile directory or artifact is ignored — fresh compile — never deserialized.

## Under the hood: fd-based verification (Unix)

A path-based ownership check is racy: an attacker can swap a symlink between the `stat` and the `open`. Wanix closes that window by verifying through file descriptors, so no path is re-resolved between the check and the read (`crates/wanix-module-cache/src/trust.rs:20-56`).

On the read path:

- The *leaf* cache directory is opened with `O_NOFOLLOW | O_DIRECTORY` and the resulting fd is `fstat`ed: it must be owned by the effective UID and carry no group/other-write bits (`0o022` clear) (`crates/wanix-module-cache/src/trust.rs:66-80`).
- The artifact is then `openat`-opened *relative to that directory fd* with `O_NOFOLLOW`, and *its* fd is `fstat`ed: it must be a regular file, owned by the effective UID, no `0o022` bits.
- The bytes are read from that same fd, so a symlink swap between check and read has no window to exploit.

The write path is hardened the same way (`crates/wanix-module-cache/src/trust.rs:170-210`): the directory is opened and `fstat`-verified through an `O_NOFOLLOW` fd, the temp artifact is created `O_CREAT | O_EXCL | O_NOFOLLOW` at mode `0o600` via `openat` relative to that fd, and the publish is a `renameat` relative to the same fd — so a swapped or symlinked leaf cannot redirect where the artifact lands. The rename is atomic, so a crashed or concurrent writer never leaves a truncated artifact behind, and the last writer wins harmlessly.

The crate's tests assert exactly these refusals: a world-writable directory, a symlinked artifact, a non-regular artifact, and a group/other-writable artifact are each treated as a cache miss and recompiled rather than deserialized (`crates/wanix-module-cache/src/lib.rs:196-296`).

## Per-runtime policy: two distinct caches

The two WASI runtimes share one implementation but pin separate locations, so they never collide. qjs uses the `WANIX_QJS_CACHE_DIR` override and the `qjs-module-cache` subdir (`crates/wanix-qjs/src/bundled.rs:16-40`); the wasm runner uses `WANIX_WASM_CACHE_DIR` and the `wasm-module-cache` subdir (`crates/wanix-wasm/src/cache.rs:16-37`). Both subdirs live under the per-user cache root.

The env override is an explicit operator opt-in to a trusted path — the operator asserts the supplied directory's ancestors are owner-private — and it is still subject to the same fd-based verification. A misconfigured override forfeits the speedup; it never causes a hostile artifact to be deserialized (`crates/wanix-module-cache/src/dir.rs:25-29`). The shared primitive lives in `wanix-module-cache`, which depends only on Wasmtime, `anyhow`, `sha2`, and (on Unix) `rustix`, with no QuickJS, linker, or task coupling — one audited trust boundary instead of two divergent copies.

## See also

- [Wasmtime as the substrate](/concepts/wasmtime-as-substrate) — why compiled modules can be serialized and deserialized at all.
- [The qjs task](/concepts/qjs-task) — the runtime whose ~550 ms cold start this cache removes.
- [The compiled wasm task driver](/concepts/compiled-wasm-task-driver) — the second runtime that shares this same cache.
- [Two tiers, one substrate](/concepts/two-tiers-one-substrate) — interpreted qjs and compiled wasm on a single Wasmtime engine.

## Status / honest limits

- **Strong verification is Unix-only.** The fd-based `O_NOFOLLOW` + `fstat` checks need a portable fd-ownership model; on non-Unix platforms the check degrades to "is a directory" and leans entirely on the per-user default location (`crates/wanix-module-cache/src/trust.rs:82-88`, `:153-156`). A non-Unix host that cannot place the cache under an owner-private location is out of the threat model.
- **Leaf + artifact, not every ancestor.** Verification covers the leaf directory and the artifact file, not every ancestor directory on each read. The cache relies on the owner-private-ancestor assumption: the default location lives under an owner-controlled per-user cache root (`crates/wanix-module-cache/src/lib.rs:41-47`, `crates/wanix-module-cache/src/dir.rs:10-29`). A writable ancestor can still swap the leaf, but the swapped-in leaf must itself pass the owner-private fd checks to be trusted.
- **The key is not a content seal.** The sha256 authenticates the input wasm, not the cached bytes, which is exactly why the cache is advisory and why the directory itself must be trusted. A failed check is always a recompile, never an execution of untrusted bytes.
