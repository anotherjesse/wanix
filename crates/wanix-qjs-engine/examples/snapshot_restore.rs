use anyhow::Result;
use rust_wasi_quickjs::{QuickJsHostConfig, QuickJsModule};
use std::path::PathBuf;
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;
    let create_config = QuickJsHostConfig::new().with_clock_time_ns(1_700_000_000_000_000_000);

    let mut runtime = module.create_runtime_with_host_config(create_config)?;
    runtime.eval_discard(
        r#"
        globalThis.counter = 41;
        queueMicrotask(() => {
          globalThis.counter += 1;
        });
        "#,
    )?;

    let jobs = runtime.execute_pending_jobs_with_limit(8)?;
    assert_eq!(jobs, 1);
    assert_eq!(runtime.eval_number("counter")?, 42.0);
    assert_eq!(runtime.eval_number("Date.now()")?, 1_700_000_000_000.0);

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    drop(runtime);

    let restore_config = QuickJsHostConfig::new().with_clock_time_ns(1_800_000_000_000_000_000);

    let mut restored =
        module.restore_runtime_from_bytes_with_host_config(&snapshot_bytes, restore_config)?;

    assert_eq!(restored.eval_number("counter")?, 42.0);
    assert_eq!(restored.eval_number("Date.now()")?, 1_800_000_000_000.0);

    println!(
        "restored counter={} Date.now()={}",
        restored.eval_number("counter")?,
        restored.eval_number("Date.now()")?
    );

    Ok(())
}
