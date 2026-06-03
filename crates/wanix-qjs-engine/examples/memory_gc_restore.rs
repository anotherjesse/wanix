use anyhow::{Result, ensure};
use rust_wasi_quickjs::QuickJsModule;
use std::path::PathBuf;
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;

    let mut runtime = module.create_runtime()?;
    runtime.set_gc_threshold(512 * 1024)?;
    runtime.eval_discard(
        r#"
        globalThis.cache = Array.from(
            { length: 2000 },
            (_, index) => ({ index, payload: "x".repeat(64) })
        );
        "#,
    )?;
    let live = runtime.memory_usage()?;

    runtime.eval_discard("globalThis.cache = undefined")?;
    runtime.run_gc()?;
    let collected = runtime.memory_usage()?;
    ensure!(
        collected.obj_count < live.obj_count,
        "expected GC to collect unreachable objects: live={}, collected={}",
        live.obj_count,
        collected.obj_count
    );

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    drop(runtime);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    restored.set_gc_threshold(50 * 1024)?;
    restored.run_gc()?;
    assert_eq!(restored.eval_number("6 * 7")?, 42.0);

    println!(
        "memory usage: objects {} -> {}, restored threshold={} bytes",
        live.obj_count,
        collected.obj_count,
        restored.gc_threshold()?
    );
    Ok(())
}
