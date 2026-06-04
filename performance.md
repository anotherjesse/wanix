# Wanix Rust qjs — Performance

Measurements of the Rust `qjs` task path: `rust -> wasmtime/wasi -> quickjs`.
The question throughout: how much overhead does that stack add over the work
itself, for cold-start, steady-state execution, memory, and concurrency.

All numbers are from an Apple Silicon dev machine, `--release`, and will vary by
host. Reproduce with the throwaway examples noted below; they are the source of
every figure here.

- `crates/wanix-qjs-engine/examples/startup_bench.rs` — compile vs cache,
  in-process run cost, memory.
- `crates/wanix-qjs-engine/examples/concurrency_bench.rs` — N concurrent async
  sleeps in one process.

```sh
cargo run --release -p wanix-qjs-engine --example startup_bench
THREADS=100 SLEEP_MS=10000 /usr/bin/time -l \
  cargo run --release -p wanix-qjs-engine --example concurrency_bench
```

## TL;DR

- A qjs CLI invocation was ~545 ms cold-start, **~100% of it Wasmtime
  cranelift-compiling** the 1.7 MiB QuickJS WASM. JS workload was noise.
- Caching the compiled module (deserialize instead of compile) makes that
  ~0.5 ms — a **~1000x** drop in the dominant cost, and also cuts peak RSS ~5x.
- The engine itself is cheap: a fresh runtime is ~150 µs, a JS eval ~1 µs, and a
  live runtime costs ~0.25 MiB resident.
- Per-invocation cost is now dominated by **OS process spawn (~18 ms, ~10 MiB)**,
  not the stack. For throughput/concurrency, run tasks in a resident host.
- 1000 concurrent async `sleepAsync` timers run in one process in ~10 s wall at
  ~0.4 MiB each; the equivalent 1000 OS processes would be ~10 GiB.

## Cold-start: where the time goes

Per-process wall-clock of `wanix-rust qjs <file>` was flat regardless of script
(noop / one `puts` / the demo all ~545–558 ms): the JS work is not the cost.

Decomposed (`startup_bench`, part 1):

| step                  | time     |
| --------------------- | -------- |
| `Module::new` compile | ~555 ms  |
| `Module::deserialize` | ~0.5 ms  |
| **speedup**           | **~1000x** |

Artifact is 4.6 MiB on disk vs 1.7 MiB wasm. So essentially the entire
cold-start is cranelift compiling the QuickJS module every process.

### Fix: cache the compiled module

`QuickJsModule::from_bytes_cached` writes a Wasmtime-serialized artifact keyed by
the wasm SHA-256, and deserializes it on later loads. `wanix-qjs`'
`from_bundled_wasm` uses it under `bundled_module_cache_dir()`
(`WANIX_QJS_CACHE_DIR` override, else a temp subdir). The cache is advisory: a
missing, stale, or engine-incompatible artifact transparently recompiles, so a
Wasmtime upgrade just recompiles under a new key.

End-to-end `wanix-rust qjs examples/qjs-demo.js`:

| run                 | wall      |
| ------------------- | --------- |
| before (no cache)   | ~545 ms   |
| cold (compile+write)| ~976 ms   |
| **warm (cached)**   | **~18 ms** |

## Steady-state: the engine is cheap

In one process with the module already loaded (`startup_bench`, part 2):

| operation                              | cost       |
| -------------------------------------- | ---------- |
| fresh runtime instantiate + trivial eval | ~150 µs/run |
| reuse one runtime, eval `1 + 1`          | ~1 µs/eval  |

So `rust -> wasmtime/wasi -> quickjs` adds almost nothing for no-op-class work
once the module is resident. The cost ladder for one `1 + 1`:

```
fork a process (warm cache)   ~18,000 µs   <- current CLI per-invocation
  deserialize module             ~600 µs   (paid once if resident)
  instantiate fresh runtime      ~150 µs
  eval JS in a live runtime        ~1 µs
```

The takeaway: the disk cache removed the compile, but per-invocation is now
dominated by **OS process spawn**, not the engine.

## Memory

`startup_bench` part 3 plus `/usr/bin/time -l` on real runs.

| thing                                   | size        |
| --------------------------------------- | ----------- |
| QuickJS heap per runtime (malloc / used)| 9 / 69 KiB  |
| host footprint per live runtime         | **~0.25 MiB** (wasm linear mem + Wasmtime instance) |
| deserialized module artifact            | 4.5 MiB (shared across instances) |
| binary on disk                          | 17 MiB      |

The cache also cuts **peak memory ~5x**, because cranelift is the memory hog:

| qjs run path           | max RSS | peak footprint |
| ---------------------- | ------- | -------------- |
| cold (cranelift compile) | 77 MiB  | 25 MiB        |
| warm (deserialize)       | 15 MiB  | 11 MiB        |

## Concurrency

100 OS processes each running a blocking `os.sleep(10000)`:

| metric                | value   |
| --------------------- | ------- |
| wall (all 100)        | 10.11 s (fully parallel) |
| summed RSS            | ~1166 MiB (~11.7 MiB each, incl. shared pages) |
| physical mem delta    | ~989 MiB (**~9.9 MiB marginal each**) |

So process-per-task scales perfectly in time but is memory-bound: ~10 MiB of
real memory per concurrent task, i.e. only a few hundred per GiB.

The engine path is far lighter. `os.sleepAsync` is a real async timer driven by
the event loop (`execute_event_loop_with_wait_budget` returns `Wait(delay)` for
future timers). `concurrency_bench` compiles the module once and runs N async
sleeps across N worker threads in **one process**:

| concurrent async sleeps | wall    | max RSS | peak footprint |
| ----------------------- | ------- | ------- | -------------- |
| 1                       | 10.01 s | 73 MiB  | 25 MiB         |
| 100                     | 10.02 s | 106 MiB | 43 MiB         |
| 1000                    | 10.13 s | 441 MiB | 383 MiB        |

(The ~73 MiB single-thread baseline includes an in-process cranelift compile;
the resident/cached path baselines at ~15 MiB. The clean number is the
**marginal ~0.2–0.4 MiB per concurrent runtime**.)

Side by side, 100 concurrent 10 s sleeps:

| model                         | wall   | memory        | per task   |
| ----------------------------- | ------ | ------------- | ---------- |
| 100 OS processes (blocking)   | 10.1 s | ~989 MiB phys | ~9.9 MiB   |
| 100 in-process async (1 proc) | 10.0 s | ~43 MiB peak  | ~0.18 MiB  |

**~25–50x less memory** for the resident multi-runtime model, same wall time.
1000 concurrent async sleeps stay at ~10 s and 383 MiB; 1000 processes would be
~10 GiB.

So no — the stack is not slow or expensive at massive concurrency, *provided*
tasks share a resident host rather than fork per call. The expense is the OS
process, not `wasmtime/wasi -> quickjs`.

### Known limitation: thread-per-task, not a single-thread reactor

Today the public event-loop pump blocks a host thread for the duration of a
future timer (`execute_event_loop_with_wait_budget` calls `thread::sleep`). So
in-process concurrency means one OS thread per waiting task, which works to ~1000
here but is ultimately thread-bound. A true single-thread reactor — pump all
runtimes with a zero budget (each returns `Wait(delay)`), sleep the minimum delay
once, then advance every runtime's deterministic clock and re-pump — needs the
clock-advance primitive (`advance_clock_time_by`, currently `pub(crate)`) exposed
on the public runtime API. That is the next lever for cheap massive concurrency
without thread overhead.

## Open follow-ups

- Expose a non-blocking clock-advance so a host can drive many runtimes on one
  thread (single-thread reactor) instead of one thread per waiting task.
- Make the `serve` daemon a resident qjs task host that shares one deserialized
  module, so real workloads pay ~150 µs / ~0.25 MiB per task, not ~18 ms / ~10 MiB.
- These figures cover no-op/sleep workloads. CPU-bound and allocation-heavy guest
  JS still need their own benchmarks; the bounded event-loop and GC policies in
  `wanix-qjs` exist for exactly that and are not yet measured here.
```
