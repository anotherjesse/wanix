# ADR 0002: WASI Task Runtime Boundary (QuickJS and compiled wasm)

## Status

Accepted

> Scope note: this ADR began as the QuickJS/WASI task boundary, but the boundary
> it records is runtime-agnostic and now also governs the compiled-`wasm32-wasi`
> task driver. The file name is kept for ADR index stability; the contract below
> covers any WASI task runtime. `.wasm` is a first-class task kind alongside
> `.js`, with the same fd-mirroring contract.

## Context

QuickJS was the first serious Wanix task runtime outside Chrome, and the
compiled-`wasm32-wasi` runner is now the second. Both run in Wasmtime as WASI
guests, but neither may become a second process model and neither may inherit
host WASI filesystem or fd semantics.

The durable boundary is the split between Wanix-owned process state and the
runtime crate's engine mechanics. Wanix owns task identity, namespace, cwd,
argv/env, stdio, fd tables, service files, terminal attachments, exit state,
WASI syscall semantics, and host execution policy, for *any* WASI task runtime.
The runtime crate owns engine mechanics: runtime instantiation, guest memory
decoding, fixture/module loading, snapshots, and provider plumbing. QuickJS
(`wanix-qjs`/`wanix-qjs-engine`) and compiled wasm (`wanix-wasm`) are two
runtime crates on the same Wanix-owned side of this boundary.

## Decision

Treat each WASI runtime as a real Wanix task driver. `qjs` (`.js`) and `wasm`
(`.wasm`) are first-class task kinds; a `.wasm` cmd auto-starts through its
driver's `check`/`start` exactly as a `.js` cmd does, sharing one
`Namespace`/VFS:

- `#task/new/<kind>` allocates task identity and service files; a `.wasm` task
  is selected by the wasm driver's `check` (program ends with `.wasm`) the same
  way a `.js` task is selected by qjs.
- `cmd`, `env`, and `dir` are Wanix task metadata and flow into the guest
  through argv/env/cwd, not through runtime-specific globals.
- fd 0/1/2 and later fds are Wanix task fds that may be backed by memory files,
  namespace files, host mounts, terminal devices, or service-file handoffs.
- fd binds capture shared open-file handles so service-file handoffs remain
  usable after the source task closes its fd.
- exit status is observable through Wanix task state (`Task::set_exit`).

`wanix-wasi` owns the Preview 1 syscall semantics used by Rust Wanix tasks. It
is backed by Wanix namespaces, task fds, explicit preopens, and Wanix filesystem
traits rather than by host WASI filesystem semantics. Its contract covers root
and cwd mapping, service paths such as `#task` and `#term`, fd operations,
rights projection, path and metadata operations, readiness, clocks/timers, and
deliberate unsupported errors for behavior that has no Wanix contract.

When a WASI task (QuickJS or wasm) exposes a guest fd as Wanix-observable state,
the adapter must mirror that fd through the Wanix task fd table and release it
when the guest closes it. Directory fds may stay WASI-internal until a Wanix
task fd contract needs them. This fd-mirroring contract is shared across both
runtimes (the wasm driver builds its live config through the same
`wanix-wasi` task config path as qjs), not reimplemented per runtime.

The checked-in QuickJS WASI fixture is a compatibility boundary for guest
modules and imports. Guest code should use `qjs:std`, `qjs:os`, `scriptArgs`,
stdio, environment, and service files. Engine-level read-only virtual files may
remain only as isolated fixture support; Wanix runtime paths must use live
Wanix-backed WASI providers.

Live Wanix-backed WASI providers are runtime host state attached through create
or restore options. They do not belong inside deterministic, cloneable
`QuickJsHostConfig` fixture policy.

QuickJS snapshots are VM images, not serialized Wanix task checkpoints. Restore
must explicitly reattach Wanix host state: namespaces, mounts, cwd, argv/env,
stdio, task fds, live WASI providers, clocks/random policy, terminal
attachments, execution limits, and exit observation. Open dynamic descriptors
block snapshot unless a future decision defines selected serializable virtual
fd state.

QuickJS timers, promise jobs, ready-IO handlers, interrupt callbacks, and heap
limits are bounded host execution policy. CLI and serve surfaces may expose
those knobs for deterministic demos and tests, but they are not a general
scheduler, signal system, cancellation model, or task checkpoint format.

### Compiled-artifact cache (qjs and wasm runtimes)

Cranelift-compiling the ~1.7 MiB bundled QuickJS fixture dominates qjs cold
start (~550 ms, ~100% of the cost). The bundled module is therefore cached on
disk as a Wasmtime-serialized artifact and deserialized on warm starts (~0.5 ms,
~1000x faster; also ~5x lower peak RSS). This is an accepted runtime decision,
not just a test fixture optimization. The standalone `.wasm` task runner
(`WasiRunner::from_bytes_cached`) uses the **same** cache for the same reason:
recompiling a non-trivial guest on every task start is wasted Cranelift work.

- **Cache key = `sha256(wasm-bytes)`.** It authenticates the *input wasm*, so a
  changed fixture lands under a new key.
- **Advisory, recompile-on-mismatch.** A missing, stale, or unreadable artifact
  falls back to a fresh compile. Wasmtime embeds its own engine/version marker
  in the artifact and rejects mismatches on `deserialize`, so an engine upgrade
  recompiles automatically under the same key.
- **Cache-dir trust boundary.** `deserialize` is `unsafe` and equivalent to
  loading native code, and the sha256 key does *not* authenticate the cached
  artifact bytes. The cache directory is therefore a trust boundary: the default
  is a per-user, owner-private directory (`0o700`), never a shared world-writable
  temp dir. `WANIX_QJS_CACHE_DIR` is an explicit operator opt-in to a trusted
  path and is still subject to the same verification. An untrusted or hostile
  cache directory or artifact is ignored (fresh compile), never deserialized.
- **Hardened fd-based verification (Unix).** Verification is done through file
  descriptors so it cannot be raced or symlink-swapped past. The *leaf* cache
  directory is opened `O_NOFOLLOW | O_DIRECTORY` and the resulting fd is
  `fstat`ed (owner == effective UID, no `0o022` write bits); the artifact is then
  `openat`-opened relative to that directory fd with `O_NOFOLLOW`, its fd is
  `fstat`ed (regular file, owner == euid, no `0o022` bits), and the bytes are
  read from that same fd — no path is re-resolved between check and read (no
  TOCTOU window, no symlink follow). The **write** path is hardened the same way:
  the directory is opened+`fstat`-verified through an `O_NOFOLLOW` fd, the temp
  artifact is created `O_CREAT | O_EXCL | O_NOFOLLOW` at `0o600` via `openat`
  relative to that fd, and the publish is a `renameat` relative to the same fd —
  so a swapped/symlinked leaf cannot redirect where the artifact is written. This
  verifies the leaf directory and the artifact file; it does **not** walk every
  ancestor, relying instead on the owner-private-ancestor assumption documented in
  each runtime's cache-policy module (the default location lives under an
  owner-controlled per-user cache root). On non-Unix platforms, with no portable
  fd-ownership model, both paths degrade to "is a directory" — a weaker guarantee,
  so a non-Unix host that cannot place the cache under an owner-private location is
  out of the threat model — and the per-user default location carries the trust.
- **One shared implementation.** The cache primitive (`load_or_compile` plus the
  fd-based verification, atomic write, and owner-private-dir resolver) lives in a
  small standalone `wanix-module-cache` crate that depends only on Wasmtime,
  `anyhow`, `sha2`, and (on Unix) `rustix`. Both `wanix-qjs-engine` and
  `wanix-wasm` depend on it, so the trust boundary has a single audited
  implementation rather than two divergent copies. The crate carries no QuickJS,
  linker, or task coupling, so neither runtime crate is forced to depend on the
  other or on the WASI linker just to share the cache.
- **Per-runtime policy.** Each runtime pins its own default location: qjs uses
  `WANIX_QJS_CACHE_DIR` and the `qjs-module-cache` subdir; the wasm runner uses
  `WANIX_WASM_CACHE_DIR` and the `wasm-module-cache` subdir (both under the
  per-user cache root, distinct so the two runtimes never collide). The env
  override in each case is an explicit operator opt-in to a trusted path, still
  subject to the same fd-based verification.

## Consequences

JavaScript running through `qjs:std`, `qjs:os`, `scriptArgs`, stdio, and
`#task` service files reaches Wanix-owned task, filesystem, fd, and lifecycle
semantics without `globalThis.Wanix` helpers and without a read-only virtual
projection bridge.

Specific syscall coverage, fixture changes, bounded-execution proofs, and demo
milestones belong in crate docs, examples, tests, and commit messages. Update
this ADR only when the durable task runtime boundary changes.
