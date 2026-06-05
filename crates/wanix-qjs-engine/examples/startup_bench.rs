//! Throwaway micro-bench: where does qjs cold-start time go?
//!
//! Part 1 times Wasmtime compiling the bundled QuickJS WASM vs deserializing a
//! precompiled artifact, to size the win from caching the compiled module.
//!
//! Part 2 stays inside one process with the module loaded once and times the
//! per-run cost that the disk cache does NOT remove: instantiating a fresh
//! QuickJS runtime per run vs reusing one runtime and only re-evaluating JS.
//!
//! Part 3 reports memory: QuickJS heap accounting for one runtime, and (with
//! `WANIX_BENCH_HOLD=N`) holds N live runtimes so an external RSS probe such as
//! `/usr/bin/time -l` can size the host footprint per concurrent instance.

use rust_wasi_quickjs::QuickJsModule;
use std::time::Instant;
use wasmtime::{Engine, Module};

const WASM: &[u8] = rust_wasi_quickjs::QUICKJS_WASM_FIXTURE;

fn main() -> anyhow::Result<()> {
    let engine = Engine::default();

    // Compile from raw wasm bytes (current per-process path).
    let mut compile_ms = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let _m = Module::new(&engine, WASM)?;
        compile_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }

    // Precompile once, then deserialize the artifact (proposed cached path).
    let artifact = engine.precompile_module(WASM)?;
    let mut deser_ms = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        // SAFETY: artifact came from this same engine/version.
        let _m = unsafe { Module::deserialize(&engine, &artifact)? };
        deser_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }

    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    println!("wasm bytes        : {} KiB", WASM.len() / 1024);
    println!("artifact bytes    : {} KiB", artifact.len() / 1024);
    println!(
        "Module::new       : {:.1} ms (mean of 5)",
        mean(&compile_ms)
    );
    println!("Module::deserialize: {:.1} ms (mean of 5)", mean(&deser_ms));
    println!(
        "speedup           : {:.0}x",
        mean(&compile_ms) / mean(&deser_ms)
    );

    // Part 2: in-process, module loaded once. What does each run still cost?
    let module = QuickJsModule::from_bytes(&engine, WASM)?;
    const RUNS: usize = 200;

    // Fresh runtime per run: full QuickJS instantiate + a trivial eval.
    let t = Instant::now();
    for _ in 0..RUNS {
        let mut rt = module.create_runtime()?;
        rt.eval_discard("1 + 1")?;
    }
    let fresh_us = t.elapsed().as_secs_f64() * 1e6 / RUNS as f64;

    // Reuse one runtime: only the JS eval repeats.
    let mut rt = module.create_runtime()?;
    let t = Instant::now();
    for _ in 0..RUNS {
        rt.eval_discard("1 + 1")?;
    }
    let reuse_us = t.elapsed().as_secs_f64() * 1e6 / RUNS as f64;

    println!("--- in-process, module loaded once (n={RUNS}) ---");
    println!("fresh runtime + eval: {fresh_us:.1} us/run");
    println!("reuse runtime, eval : {reuse_us:.1} us/run");

    // Part 3: memory. QuickJS heap accounting for one fresh runtime.
    let mut probe = module.create_runtime()?;
    probe.eval_discard("1 + 1")?;
    let usage = probe.memory_usage()?;
    println!("--- memory ---");
    println!(
        "QuickJS heap malloc_size : {} KiB",
        usage.malloc_size / 1024
    );
    println!(
        "QuickJS memory_used_size : {} KiB",
        usage.memory_used_size / 1024
    );

    // Hold N live runtimes so an external RSS probe (/usr/bin/time -l) can size
    // the host footprint per concurrent instance. WANIX_BENCH_HOLD=N enables it.
    if let Some(n) = std::env::var("WANIX_BENCH_HOLD")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        let mut held = Vec::with_capacity(n);
        for _ in 0..n {
            let mut rt = module.create_runtime()?;
            rt.eval_discard("1 + 1")?;
            held.push(rt);
        }
        println!("holding {} live runtimes", held.len());
        // Keep them alive past the RSS sample.
        std::hint::black_box(&held);
    }
    Ok(())
}
