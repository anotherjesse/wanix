---
title: Performance & scaling
slug: reference/performance
pageType: reference
oneLiner: The rooms-not-houses cost model and what the bench actually measures.
audience: [newcomer, developer, visionary]
tags: [shipped, caveat, scaling, two-tiers, local-trust-only]
sourceRefs:
  - performance.md:24-35
  - performance.md:99-104
  - performance.md:163-191
  - performance.md:210-231
  - docs/scaling-eli5.md:9-23
  - docs/scaling-eli5.md:89-95
  - performance.md:287-289
seeAlso:
  - concepts/rooms-not-houses
  - concepts/safe-for-untrusted-not-claimable
  - concepts/two-tiers-one-substrate
  - concepts/wasmtime-as-substrate
  - concepts/bounded-execution-policy
prerequisites: []
usedInFlows: []
honestLimits:
  - There are no hard CPU/memory limits yet; isolation is cheap and scalable, not a sandbox for arbitrary untrusted code.
  - Every number here is from one Apple Silicon / 18-core dev machine in --release; it will vary by host.
  - QuickJS is an interpreter, so per-task JS compute is ~50-65x off native; Wanix sells isolation, not raw speed.
canonicalCaveatFor: []
---

# Performance & scaling

The rooms-not-houses cost model and what the bench actually measures.

Wanix's claim is narrow and measurable: it runs **thousands of isolated programs
cheaply at the same time**, scaling smoothly up to whatever the hardware can do.
It does *not* make any one program fast — the interpreted-JS tier is slower than
native, on purpose, in exchange for tiny instant rooms. This page states the cost
model in plain terms, shows the numbers the throwaway benchmarks produce, and is
blunt about the one place the marketing could overclaim: cheap isolation is not
yet a sandbox for arbitrary untrusted code. Every figure here is reproduced from
`performance.md` and `docs/scaling-eli5.md`, run on one 18-core dev machine in
`--release`; they will vary by host.

## Rooms, not houses

A normal OS process is a *house*: safe — nobody walks into yours — but expensive,
about **10 MiB of real memory just to exist** before it does any work
(`performance.md:99-104`). A thousand little programs that way is roughly 10 GiB,
mostly empty houses.

Wanix runs each task as a WebAssembly instance — a **locked room in a shared
building**. A room is private (one room's code can't see another's) and reaches
only the doors — files, devices — you bind into its namespace. A live runtime
costs **~0.25 MiB** resident (`performance.md:99-104`): wasm linear memory plus
the Wasmtime instance, roughly 40x smaller than a house. The same thousand
programs fit in one building in a few hundred MiB. This is the
[rooms-not-houses](/concepts/rooms-not-houses) model, and it is the whole point of
[Wasmtime as the substrate](/concepts/wasmtime-as-substrate): namespaces and
instances are cheap to create, so isolation stops being a thing you ration.

## The scaling-eli5 model

The plain-language version (`docs/scaling-eli5.md:9-23`): a *small* number of
buildings — chosen for blast radius and bad-neighbor containment, **not one per
user** — each building using all your cores, and inside each, thousands of cheap
private rooms. You dial in isolation: a quick JS snippet shares a building; a
hostile or high-value tenant gets its own building or a full VM. Same security
story across the tiers; you choose how much you pay.

## What the bench actually measures

Cold-start of a one-shot `wanix qjs <file>` was ~545 ms, and **~100% of that
was Wasmtime cranelift-compiling** the 1.7 MiB QuickJS module — the JS workload
was noise. Caching the compiled artifact drops the dominant cost to ~0.5 ms
(a ~1000x fall) and cuts peak RSS ~5x; a warm cached invocation is ~18 ms
(`performance.md:24-35`). Once a module is resident the engine is nearly free: a
fresh runtime is ~150 µs, a `1 + 1` eval is ~1 µs. The lesson is that
per-invocation cost is now dominated by **OS process spawn (~18 ms, ~10 MiB)**,
not the `rust → wasmtime/wasi → quickjs` stack. For throughput, run tasks in a
resident host rather than forking per call.

### Concurrency holds flat with cores

Sleeping proves orchestration is cheap; real work proves scaling. `concurrency_bench`
with `WORKLOAD=cpu` runs a tight 20 M-iteration arithmetic loop per task on the
18-core machine (`performance.md:163-191`):

| concurrent CPU tasks | wall    | throughput  | scaling vs 1 |
| -------------------- | ------- | ----------- | ------------ |
| 1                    | 1.02 s  | 1 task/s    | 1.0x         |
| 18 (= cores)         | 1.44 s  | 13 tasks/s  | 13.2x        |
| 100                  | 7.67 s  | 13 tasks/s  | 13.7x        |
| 1000                 | 82.8 s  | 12 tasks/s  | 12.3x        |

Throughput saturates near the core count and **stays flat from 18 to 1000 tasks**
— no thrashing, no collapse. Past the core count, extra tasks queue in an orderly
line and total time grows linearly; they do not degrade each other. IO through the
full Wanix path is fast and parallel too: a single task sustains ~520k filesystem
ops/s, and 50 concurrent IO tasks reach ~9 M ops/s aggregate, because each task has
its own private filesystem.

### Two tiers, one substrate

QuickJS is a bytecode interpreter, so the same loop is **~60-65x slower than native
Rust** (`performance.md:163-191`). The fix is not a different sandbox — it is a
different *engine inside the same room*. `compute_bench` runs one pi kernel three
ways through the real paths (`performance.md:210-231`):

| same pi kernel, 20M terms | time     | vs native |
| ------------------------- | -------- | --------- |
| native Rust               | ~11.6 ms | 1.0x      |
| rust-wasm in Wanix        | ~12.8 ms | **1.10x** |
| qjs in Wanix              | ~603 ms  | ~52x      |

A Rust task compiled to `wasm32-wasi` and run as a real Wanix task is **within
~10% of native** — the WASI/instantiate/namespace path adds almost nothing.
Interpreted JS pays a ~50x tax. Concurrent, 100 tasks in one process: rust-wasm
hits ~1190 tasks/s, qjs ~19 tasks/s — ~60x the aggregate throughput for the same
compute, same sandbox, same capability boundary. This is
[two tiers, one substrate](/concepts/two-tiers-one-substrate): Tier 0 (interpreted
JS) for glue and AI-generated snippets, Tier 1 (compiled wasm) for hot paths, Tier
2 (native process / v86 VM) for full-OS or hostile-at-scale needs.

## Status / honest limits

- **No hard CPU or memory caps yet.** Per-room *memory isolation* is real today,
  but the Wasmtime epoch/fuel preemption and `ResourceLimiter`/`StoreLimits`
  linear-memory caps that would make a room safe for arbitrary untrusted code are
  not wired up (`docs/scaling-eli5.md:89-95`, `performance.md:287-289`). Keep the
  claim to "cheap, scalable isolation" — this is
  [safe-for-untrusted-not-claimable](/concepts/safe-for-untrusted-not-claimable),
  and the boundary it implies is [bounded execution policy](/concepts/bounded-execution-policy).
- **Wanix does not make the language fast.** The interpreted tier is ~50-65x off
  native by design; drop hot kernels to compiled wasm.
- **Every number is one machine.** Apple Silicon / 18-core, `--release`. Reproduce
  with `startup_bench`, `concurrency_bench`, and `compute_bench`; treat the figures
  as shape, not a spec.
- **Concurrency is thread-per-task today.** The event-loop pump blocks a host
  thread per waiting task; a single-thread reactor (the next lever for massive
  cheap concurrency) needs a non-blocking clock-advance primitive that is not yet
  on the public API (`performance.md:287-289`).

## See also

- [Rooms, not houses](/concepts/rooms-not-houses) — the cost model in full.
- [Two tiers, one substrate](/concepts/two-tiers-one-substrate) — qjs vs compiled wasm.
- [Safe-for-untrusted is not claimable](/concepts/safe-for-untrusted-not-claimable) — the missing hard limits.
- [Wasmtime as the substrate](/concepts/wasmtime-as-substrate) — why instances are cheap.
- [Bounded execution policy](/concepts/bounded-execution-policy) — where preemption and caps will live.
