# ADR 0034: WASI Timestamp NOW Uses Wanix Clock Policy

## Status

Accepted

## Context

ADR 0030 added Wanix-owned `path_filestat_set_times` and
`fd_filestat_set_times` semantics but rejected Preview 1 `ATIM_NOW` and
`MTIM_NOW` flags until Wanix chose where the "now" timestamp comes from.

The engine crate already keeps clock behavior deterministic through
`QuickJsHostConfig::clock_time_ns()`. `wanix-wasi` owns filesystem and fd
semantics, so timestamp `*_NOW` policy should live in the Wanix WASI config
rather than in the QuickJS engine or host operating system.

## Decision

Add `WasiConfig::clock_time_ns()` and `WasiConfig::with_clock_time_ns(...)`.
The default is `1_700_000_000_000_000_000`, matching the engine crate's default
clock value so standalone Wanix WASI contexts and QuickJS-hosted contexts start
from the same deterministic policy.

When `wanix-qjs` creates or restores a runtime from `QuickJsWanixConfig`, it
mirrors the `WasiConfig` clock into `QuickJsHostConfig` so QuickJS
`clock_time_get`/`Date.now()` and Wanix-owned timestamp `*_NOW` updates observe
the same task clock.

`WasiCtx::path_filestat_set_times` and `WasiCtx::fd_filestat_set_times` accept
Preview 1 `ATIM_NOW` and `MTIM_NOW` flags. When a `*_NOW` flag is present, the
affected timestamp is set to the configured `WasiConfig` clock. Explicit
timestamp flags still use the syscall timestamp argument. Absent flags preserve
the existing metadata value. Unknown or contradictory flags still return
`Errno::Inval`.

## Consequences

WASI guests can now request clock-derived timestamp updates without delegating
filesystem policy to QuickJS or the host OS. Tests can pin deterministic "now"
values by configuring `WasiConfig` directly.

This does not add wall-clock time, symlink-specific timestamp handling, or richer
ctime policy. Embedders that want real host time can choose to construct
`WasiConfig` with that value, while snapshots still contain only QuickJS/Wasm VM
memory and reattach host timestamp policy on restore.
