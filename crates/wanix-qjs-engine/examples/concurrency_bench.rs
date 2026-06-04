//! Throwaway micro-bench: in-process concurrency of async QuickJS sleeps.
//!
//! One process compiles the QuickJS module once, then spawns `THREADS` worker
//! threads that each create their own runtime and run an `os.sleepAsync` timer
//! to completion. This sizes the host footprint of N concurrent async tasks
//! sharing one module, versus N separate OS processes.
//!
//! Env knobs: `THREADS` (default 100), `SLEEP_MS` (default 10000). Wrap with
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

fn main() -> anyhow::Result<()> {
    let threads = env_usize("THREADS", 100);
    let sleep_ms = env_usize("SLEEP_MS", 10_000) as u64;

    // Compile once; every worker shares this module and its engine.
    let engine = Engine::default();
    let module = QuickJsModule::from_bytes(&engine, WASM)?;

    let src =
        format!("import * as os from \"qjs:os\";\nos.sleepAsync({sleep_ms}).then(() => {{}});\n");

    let start = Instant::now();
    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let module = module.clone();
        let src = src.clone();
        handles.push(thread::spawn(move || -> anyhow::Result<()> {
            let mut rt = module.create_runtime()?;
            rt.eval_module_discard(&src, "main.js")?;
            // Drive the async timer to completion; one wait budget per worker.
            rt.execute_event_loop_with_wait_budget(
                1_000_000,
                Duration::from_millis(sleep_ms + 2_000),
            )?;
            Ok(())
        }));
    }
    for h in handles {
        h.join().expect("worker panicked")?;
    }
    let wall = start.elapsed().as_secs_f64();

    println!("threads (concurrent async sleeps): {threads}");
    println!("sleep per task                   : {sleep_ms} ms");
    println!("wall                             : {wall:.2} s");
    Ok(())
}
