//! Throwaway micro-bench: compiled-to-wasm speed vs interpreted JS, same substrate.
//!
//! Runs the exact arithmetic kernel from `concurrency_bench` three ways:
//!   1. native Rust (scalar, black-boxed so it isn't optimized away),
//!   2. compiled wasm run on the same Wasmtime engine Wanix already uses,
//!   3. (for reference) the QuickJS number is ~1000 ms for 20M iters.
//!
//! Point: the substrate that runs the QuickJS interpreter runs *any* wasm. A
//! Rust/Go/C/Zig task compiled to wasm runs here at cranelift-JIT speed, in the
//! same sandbox, instead of ~60x slower interpreted JS — the "performance when
//! you need it" tier.
//!
//! Env: `ITERS` (default 20_000_000).

use std::time::Instant;
use wasmtime::{Engine, Instance, Module, Store};

// Same kernel as the JS/native loop: x += i * 2654435761 (wrapping u32).
const KERNEL_WAT: &str = r#"
(module
  (func (export "kernel") (param $iters i32) (result i32)
    (local $i i32) (local $x i32)
    (block $brk
      (loop $lp
        (br_if $brk (i32.ge_u (local.get $i) (local.get $iters)))
        (local.set $x
          (i32.add (local.get $x)
                   (i32.mul (local.get $i) (i32.const 2654435761))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $lp)))
    (local.get $x)))
"#;

fn native_kernel(iters: u32) -> u32 {
    let mut x: u32 = std::hint::black_box(0);
    for i in 0..iters {
        let i = std::hint::black_box(i);
        x = x.wrapping_add(i.wrapping_mul(2_654_435_761));
    }
    x
}

fn main() -> anyhow::Result<()> {
    let iters: u32 = std::env::var("ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000_000);

    // Native baseline.
    let t = Instant::now();
    let native_out = std::hint::black_box(native_kernel(iters));
    let native_ms = t.elapsed().as_secs_f64() * 1e3;

    // Compiled wasm on the same engine Wanix uses for QuickJS.
    let engine = Engine::default();
    let module = Module::new(&engine, KERNEL_WAT)?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])?;
    let kernel = instance.get_typed_func::<i32, i32>(&mut store, "kernel")?;
    // Warm once (instantiation/JIT already done), then time the call.
    let _ = kernel.call(&mut store, iters as i32)?;
    let t = Instant::now();
    let wasm_out = kernel.call(&mut store, iters as i32)? as u32;
    let wasm_ms = t.elapsed().as_secs_f64() * 1e3;

    assert_eq!(native_out, wasm_out, "wasm and native kernels must agree");

    // QuickJS reference for the same 20M-iter kernel (from concurrency_bench).
    let qjs_ref_ms = 1015.0 * (iters as f64 / 20_000_000.0);

    println!("iters                    : {iters}");
    println!("native Rust (black-boxed): {native_ms:.1} ms   (scalar, deliberately un-optimized)");
    println!("compiled wasm (Wasmtime) : {wasm_ms:.1} ms");
    println!("QuickJS interpreted (ref): {qjs_ref_ms:.0} ms");
    println!(
        "compiled wasm vs QuickJS : {:.0}x faster, same sandbox",
        qjs_ref_ms / wasm_ms.max(1e-9)
    );
    Ok(())
}
