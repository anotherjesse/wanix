use anyhow::{Context, Result};
use rust_wasi_quickjs::QuickJsModule;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;

    let mut runtime = module.create_runtime()?;
    runtime.eval_discard("globalThis.resumeMarker = 'snapshotted'")?;
    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    drop(runtime);

    let events = Arc::new(Mutex::new(Vec::new()));
    let events_for_handler = Arc::clone(&events);
    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    restored.set_promise_rejection_handler(move |event| {
        if let Ok(mut events) = events_for_handler.lock() {
            events.push((event.reason().to_string(), event.is_handled()));
        }
    })?;

    restored.eval_discard(r#"Promise.reject("post-restore rejection")"#)?;
    restored.execute_pending_jobs()?;

    let events = events
        .lock()
        .map_err(|_| anyhow::anyhow!("promise rejection event lock was poisoned"))?;
    assert!(
        events
            .iter()
            .any(|(reason, is_handled)| reason == "post-restore rejection" && !is_handled)
    );
    assert_eq!(
        restored
            .eval_string("resumeMarker")
            .context("restored marker should still be readable")?,
        "snapshotted"
    );

    println!("promise rejections after restore: {events:?}");
    Ok(())
}
