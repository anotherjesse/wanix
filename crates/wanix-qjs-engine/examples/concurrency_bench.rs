//! Throwaway micro-bench: in-process concurrency of QuickJS tasks doing real work.
//!
//! One process compiles the QuickJS module once, then spawns `THREADS` worker
//! threads that each create their own runtime and run a workload to completion.
//! This sizes the host footprint and throughput of N concurrent tasks sharing
//! one module, versus N separate OS processes.
//!
//! Workloads (`WORKLOAD`):
//!   - `sleep` : `os.sleepAsync(SLEEP_MS)` — concurrency without CPU cost.
//!   - `cpu`   : a tight arithmetic loop of `ITERS` iterations — CPU-bound work.
//!
//! For `cpu`, the bench also runs the identical loop in native Rust so the
//! per-iteration overhead of `rust -> wasmtime/wasi -> quickjs` is visible.
//!
//! Env knobs: `WORKLOAD` (default `cpu`), `THREADS` (default 100),
//! `SLEEP_MS` (default 10000), `ITERS` (default 20_000_000). Wrap with
//! `/usr/bin/time -l` to read peak RSS.

use std::thread;
use std::time::{Duration, Instant};

use rust_wasi_quickjs::QuickJsModule;
use wasmtime::Engine;

const WASM: &[u8] = rust_wasi_quickjs::QUICKJS_WASM_FIXTURE;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The same arithmetic kernel the JS workload runs, in native Rust, for an
/// apples-to-apples per-iteration overhead comparison.
fn native_kernel(iters: u64) -> u32 {
    let mut x: u32 = std::hint::black_box(0);
    for i in 0..iters {
        // black_box each input so LLVM cannot fold or vectorize the loop away,
        // keeping it a scalar loop comparable to the QuickJS interpreter loop.
        let i = std::hint::black_box(i as u32);
        x = x.wrapping_add(i.wrapping_mul(2_654_435_761));
    }
    x
}

fn main() -> anyhow::Result<()> {
    let workload = std::env::var("WORKLOAD").unwrap_or_else(|_| "cpu".to_string());
    let threads = env_usize("THREADS", 100);
    let sleep_ms = env_usize("SLEEP_MS", 10_000) as u64;
    let iters = env_usize("ITERS", 20_000_000) as u64;

    // Compile once; every worker shares this module and its engine.
    let engine = Engine::default();
    let module = QuickJsModule::from_bytes(&engine, WASM)?;

    let cpu_src = format!(
        "let x = 0;\nfor (let i = 0; i < {iters}; i++) {{ x = (x + i * 2654435761) >>> 0; }}\nglobalThis._sink = x;\n"
    );
    let sleep_src =
        format!("import * as os from \"qjs:os\";\nos.sleepAsync({sleep_ms}).then(() => {{}});\n");

    // Single-task baseline (module already loaded) for the chosen workload.
    let mut warm = module.create_runtime()?;
    let t = Instant::now();
    run_once(&mut warm, &workload, &cpu_src, &sleep_src, sleep_ms)?;
    let single = t.elapsed().as_secs_f64();

    // Native Rust baseline of the same CPU kernel.
    let native = {
        let t = Instant::now();
        std::hint::black_box(native_kernel(iters));
        t.elapsed().as_secs_f64()
    };

    // N concurrent tasks across N threads.
    let start = Instant::now();
    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let module = module.clone();
        let workload = workload.clone();
        let cpu_src = cpu_src.clone();
        let sleep_src = sleep_src.clone();
        handles.push(thread::spawn(move || -> anyhow::Result<()> {
            let mut rt = module.create_runtime()?;
            run_once(&mut rt, &workload, &cpu_src, &sleep_src, sleep_ms)
        }));
    }
    for h in handles {
        h.join().expect("worker panicked")?;
    }
    let wall = start.elapsed().as_secs_f64();

    println!("workload                 : {workload}");
    println!("threads (concurrent)     : {threads}");
    if workload == "cpu" {
        println!("iters per task           : {iters}");
        println!("native kernel (1x)       : {:.1} ms", native * 1e3);
        println!("qjs single task (1x)     : {:.1} ms", single * 1e3);
        println!(
            "qjs overhead vs native   : {:.0}x",
            single / native.max(1e-9)
        );
        let task_per_s = threads as f64 / wall;
        println!("wall for {threads:>4} tasks      : {:.2} s", wall);
        println!("throughput               : {task_per_s:.0} tasks/s");
        println!(
            "scaling vs single        : {:.1}x (ideal up to {} cores)",
            single * threads as f64 / wall,
            std::thread::available_parallelism().map_or(0, |n| n.get())
        );
    } else {
        println!("sleep per task           : {sleep_ms} ms");
        println!("wall for {threads:>4} tasks      : {:.2} s", wall);
    }
    Ok(())
}

fn run_once(
    rt: &mut rust_wasi_quickjs::QuickJsRuntime,
    workload: &str,
    cpu_src: &str,
    sleep_src: &str,
    sleep_ms: u64,
) -> anyhow::Result<()> {
    if workload == "cpu" {
        rt.eval_discard(cpu_src)?;
    } else {
        rt.eval_module_discard(sleep_src, "main.js")?;
        rt.execute_event_loop_with_wait_budget(1_000_000, Duration::from_millis(sleep_ms + 2_000))?;
    }
    Ok(())
}
