use anyhow::Result;
use rust_wasi_quickjs::QuickJsModule;
use std::path::PathBuf;
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;

    let mut runtime = module.create_runtime()?;
    runtime.eval_discard(
        r#"
        globalThis.stepResult = "not yet";
        let resolvePending;
        globalThis.pendingStep = new Promise(resolve => {
          resolvePending = resolve;
        });
        globalThis.resolvePendingStep = resolvePending;
        globalThis.pendingStep.then(value => {
          globalThis.stepResult = "completed: " + value;
        });
        "#,
    )?;
    let pre_snapshot_jobs = runtime.execute_pending_jobs_with_limit(8)?;
    assert_eq!(pre_snapshot_jobs, 0);
    assert_eq!(runtime.eval_string("stepResult")?, "not yet");

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    drop(runtime);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    restored.call_global_function_with_string("resolvePendingStep", "resumed in Rust")?;
    let jobs = restored.execute_pending_jobs_with_limit(8)?;

    assert_eq!(jobs, 1);
    assert_eq!(
        restored.eval_string("stepResult")?,
        "completed: resumed in Rust"
    );

    println!(
        "pending promise resumed after {} job: {}",
        jobs,
        restored.eval_string("stepResult")?
    );

    Ok(())
}
