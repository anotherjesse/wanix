# ADR 0005: Make Host Import Configuration Explicit

## Status

Accepted

## Context

WebAssembly linear memory snapshots preserve QuickJS heap state, but they do not
preserve Rust host state. The runtime imports host-provided clock, random,
timezone, stdio, callback, module-loading, and other resource behavior. Before
host callbacks and module loaders are implemented, the deterministic and
observable imports should already be explicit so create and restore do not
silently depend on hidden defaults.

## Decision

Expose `QuickJsHostConfig` and thread it through
`QuickJsRuntime::create_with_host_config` and
`QuickJsRuntime::restore_with_host_config`, plus the preferred module-owned
`QuickJsModule::create_runtime_with_host_config` and
`QuickJsModule::restore_runtime_with_host_config` helpers. Keep default-config
convenience wrappers for both runtime-owned and module-owned lifecycle APIs.

The initial config covers host imports that are already present:

- WASI `clock_time_get` via `clock_time_ns`.
- WASI `random_get` via a repeated `random_byte`.
- `env.host_get_timezone_offset` via `timezone_offset_seconds`.
- WASI `fd_write` via optional stdout/stderr capture, defaulting to process
  stdout/stderr inheritance, with optional per-stream retained byte limits.
- WASI read-only virtual files via explicit guest path-to-bytes mappings, as
  narrowed by ADR 0020.

Snapshots do not serialize this config. On restore, callers provide the host
config that should be attached to the resumed runtime.

## Consequences

- Restore behavior can deliberately reattach the same host policy or a different
  host policy.
- Snapshot bytes remain VM images rather than bundles of Rust callbacks, clocks,
  timers, files, random streams, stdio buffers, module loaders, or futures.
- Captured stdio is runtime host state. Restoring a snapshot attaches the
  capture policy and byte limits from the restore config and starts with fresh
  capture buffers.
- Virtual filesystem config is runtime host state. Restoring a snapshot
  attaches the restore config's immutable guest files, and snapshots reject live
  virtual file descriptors rather than silently dropping descriptor position.
- Future host callbacks and module loaders should be reattached through
  explicit config/registries with stable names instead of being captured inside
  the snapshot.
- The current random configuration is intentionally simple until a public crypto
  extension or WASI-observation API makes richer entropy tests meaningful.
