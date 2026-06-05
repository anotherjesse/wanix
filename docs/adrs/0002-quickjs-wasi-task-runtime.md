# ADR 0002: QuickJS/WASI Task Runtime Boundary

## Status

Accepted

## Context

QuickJS is the first serious Wanix task runtime outside Chrome. It runs in
Wasmtime as a WASI guest, but it must not become a second process model and it
must not inherit host WASI filesystem or fd semantics.

The durable boundary is the split between Wanix-owned process state and
QuickJS/Wasmtime engine mechanics. Wanix owns task identity, namespace, cwd,
argv/env, stdio, fd tables, service files, terminal attachments, exit state,
WASI syscall semantics, and host execution policy. The engine crate owns
runtime instantiation, guest memory decoding, fixture loading, snapshots, and
provider plumbing.

## Decision

Treat `qjs` as a real Wanix task driver:

- `#task/new/qjs` allocates task identity and service files.
- `cmd`, `env`, and `dir` are Wanix task metadata and flow into QuickJS through
  argv/env/cwd, not through JavaScript globals.
- fd 0/1/2 and later fds are Wanix task fds that may be backed by memory files,
  namespace files, host mounts, terminal devices, or service-file handoffs.
- fd binds capture shared open-file handles so service-file handoffs remain
  usable after the source task closes its fd.
- exit status is observable through Wanix task state.

`wanix-wasi` owns the Preview 1 syscall semantics used by Rust Wanix tasks. It
is backed by Wanix namespaces, task fds, explicit preopens, and Wanix filesystem
traits rather than by host WASI filesystem semantics. Its contract covers root
and cwd mapping, service paths such as `#task` and `#term`, fd operations,
rights projection, path and metadata operations, readiness, clocks/timers, and
deliberate unsupported errors for behavior that has no Wanix contract.

When a QuickJS task exposes a guest fd as Wanix-observable state, the adapter
must mirror that fd through the Wanix task fd table and release it when the
guest closes it. Directory fds may stay WASI-internal until a Wanix task fd
contract needs them.

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

### Bundled-module compiled-artifact cache

Cranelift-compiling the ~1.7 MiB bundled QuickJS fixture dominates qjs cold
start (~550 ms, ~100% of the cost). The bundled module is therefore cached on
disk as a Wasmtime-serialized artifact and deserialized on warm starts (~0.5 ms,
~1000x faster; also ~5x lower peak RSS). This is an accepted runtime decision,
not just a test fixture optimization.

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
  TOCTOU window, no symlink follow). The temp artifact is created `0o600` as
  defense-in-depth. This verifies the leaf directory and the artifact file; it
  does **not** walk every ancestor, relying instead on the owner-private-ancestor
  assumption documented in `wanix-qjs`'s `bundled` module (the default location
  lives under an owner-controlled per-user cache root). On non-Unix platforms,
  with no portable fd-ownership model, the check degrades to "is a directory" — a
  weaker guarantee — and the per-user default location carries the trust.
- **Non-goal.** This is the *bundled QuickJS* module cache only. A compiled-
  module cache for the standalone `.wasm` runner is deliberately out of scope
  here and belongs to the wasm-runner phase.

## Consequences

JavaScript running through `qjs:std`, `qjs:os`, `scriptArgs`, stdio, and
`#task` service files reaches Wanix-owned task, filesystem, fd, and lifecycle
semantics without `globalThis.Wanix` helpers and without a read-only virtual
projection bridge.

Specific syscall coverage, fixture changes, bounded-execution proofs, and demo
milestones belong in crate docs, examples, tests, and commit messages. Update
this ADR only when the durable task runtime boundary changes.
