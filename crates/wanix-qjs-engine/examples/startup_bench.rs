//! Throwaway micro-bench: where does qjs cold-start time go?
//!
//! Times Wasmtime compiling the bundled QuickJS WASM vs deserializing a
//! precompiled artifact, to size the win from caching the compiled module.

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
    Ok(())
}
