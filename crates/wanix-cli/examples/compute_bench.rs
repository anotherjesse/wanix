//! Apples-to-apples CPU benchmark: the SAME pi kernel (Leibniz series) run three
//! ways, single and concurrent, all measured end-to-end through Wanix.
//!
//! 1. native Rust — in-process baseline.
//! 2. rust-wasm in Wanix — compiled wasm32-wasi task via `wanix_wasm::WasiRunner`
//!    (real WASI imports + namespace, not bare Wasmtime).
//! 3. QuickJS in Wanix — interpreted JS task via `QuickJsRunner`.
//!
//! Paths 2 and 3 are the actual Wanix task paths: instantiate + run + stdio.
//!
//! Env: ITERS (pi terms, default 20_000_000), THREADS (concurrency, default 100).
//! Run: WANIX_QJS_CACHE_DIR=/tmp/wqjs-cache \
//!        cargo run --release -p wanix-cli --example compute_bench

use std::sync::Arc;
use std::thread;
use std::time::Instant;

use wanix_qjs::{QuickJsRunner, QuickJsWanixConfig};
use wanix_vfs::Namespace;
use wanix_wasi::WasiConfig;
use wanix_wasm::{CaptureFile, WasiRunner};

const RUST_GUEST: &[u8] = include_bytes!("../../wanix-wasm/fixtures/rust-guest.wasm");

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Native Rust kernel; black-boxed so the loop is not optimized away.
fn native_pi(n: u64) -> f64 {
    let mut acc = 0.0f64;
    let mut sign = 1.0f64;
    for k in 0..std::hint::black_box(n) {
        acc += sign / (2 * k + 1) as f64;
        sign = -sign;
    }
    std::hint::black_box(4.0 * acc)
}

fn run_rust_wasm(runner: &WasiRunner, iters: u64) -> (f64, i32) {
    let stdout = CaptureFile::new();
    let config = WasiConfig::new(Namespace::new())
        .with_args(["guest", "--pi", &iters.to_string()])
        .with_stdout(Box::new(stdout.clone()), "stdout");
    let exit = runner.run(config).expect("rust-wasm task ran");
    (stdout.contents().trim().parse().unwrap_or(f64::NAN), exit)
}

fn qjs_pi_source(iters: u64) -> String {
    format!(
        r#"import * as std from "qjs:std";
let acc = 0, sign = 1;
for (let k = 0; k < {iters}; k++) {{ acc += sign / (2 * k + 1); sign = -sign; }}
std.out.puts((4 * acc).toFixed(10) + "\n");
std.out.flush();
"#
    )
}

fn run_qjs(runner: &QuickJsRunner, iters: u64) -> f64 {
    let stdout = CaptureFile::new();
    let wasi = WasiConfig::new(Namespace::new()).with_stdout(Box::new(stdout.clone()), "stdout");
    runner
        .run_source_with_wanix_config(&qjs_pi_source(iters), QuickJsWanixConfig::new(wasi))
        .expect("qjs task ran");
    stdout.contents().trim().parse().unwrap_or(f64::NAN)
}

fn ms(f: impl FnOnce()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1e3
}

/// Runs `count` closures across `count` threads, returns wall seconds.
fn concurrent(count: usize, body: impl Fn(usize) + Sync) -> f64 {
    let t = Instant::now();
    thread::scope(|s| {
        for i in 0..count {
            let body = &body;
            s.spawn(move || body(i));
        }
    });
    t.elapsed().as_secs_f64()
}

fn main() {
    let iters = env_usize("ITERS", 20_000_000) as u64;
    let threads = env_usize("THREADS", 100);

    let rust = Arc::new(WasiRunner::from_bytes(RUST_GUEST).expect("compile rust guest"));
    let qjs = Arc::new(QuickJsRunner::from_bundled_wasm().expect("qjs runner"));

    println!("pi kernel: Leibniz series, {iters} terms\n");

    // --- single run ---
    let mut native_pi_val = 0.0;
    let native_ms = ms(|| native_pi_val = native_pi(iters));
    let mut rust_pi_val = 0.0;
    let rust_ms = ms(|| rust_pi_val = run_rust_wasm(&rust, iters).0);
    let mut qjs_pi_val = 0.0;
    let qjs_ms = ms(|| qjs_pi_val = run_qjs(&qjs, iters));

    println!("--- single run ---");
    println!("native rust         : {native_ms:8.1} ms   (pi = {native_pi_val:.6})");
    println!("rust-wasm in Wanix  : {rust_ms:8.1} ms   (pi = {rust_pi_val:.6})");
    println!("qjs in Wanix        : {qjs_ms:8.1} ms   (pi = {qjs_pi_val:.6})");
    println!(
        "rust-wasm vs native : {:.2}x   |   qjs vs native : {:.0}x   |   qjs vs rust-wasm : {:.0}x",
        rust_ms / native_ms.max(1e-9),
        qjs_ms / native_ms.max(1e-9),
        qjs_ms / rust_ms.max(1e-9),
    );
    assert!(
        (native_pi_val - rust_pi_val).abs() < 1e-6,
        "native vs rust-wasm pi mismatch"
    );
    assert!(
        (native_pi_val - qjs_pi_val).abs() < 1e-6,
        "native vs qjs pi mismatch"
    );
    println!("(all three computed the same pi to 1e-6)\n");

    // --- concurrent ---
    println!("--- {threads} concurrent tasks ---");
    let rust_wall = concurrent(threads, |_| {
        run_rust_wasm(&rust, iters);
    });
    println!(
        "rust-wasm in Wanix  : {rust_wall:6.2} s wall   {:.0} tasks/s",
        threads as f64 / rust_wall
    );
    let qjs_wall = concurrent(threads, |_| {
        run_qjs(&qjs, iters);
    });
    println!(
        "qjs in Wanix        : {qjs_wall:6.2} s wall   {:.0} tasks/s",
        threads as f64 / qjs_wall
    );
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    println!(
        "(speedup vs single: rust-wasm {:.1}x, qjs {:.1}x on {cores} cores)",
        rust_ms * threads as f64 / 1e3 / rust_wall,
        qjs_ms * threads as f64 / 1e3 / qjs_wall,
    );
}
