use anyhow::Result;
use rust_wasi_quickjs::{QuickJsHostConfig, QuickJsModule, Snapshot};
use std::path::PathBuf;
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;

    let create_config = QuickJsHostConfig::new()
        .with_clock_time_ns(1_700_000_000_000_000_000)
        .with_timezone_offset_seconds(3_600);

    let mut runtime = module.create_runtime_with_host_config(create_config)?;
    runtime.eval_discard(
        r#"
        globalThis.policyState = "snapshotted";
        globalThis.createOffset = new Date(0).getTimezoneOffset();
        "#,
    )?;
    assert_eq!(runtime.eval_number("Date.now()")?, 1_700_000_000_000.0);
    assert_eq!(runtime.eval_number("createOffset")?, -60.0);

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    let metadata = Snapshot::metadata_from_bytes(&snapshot_bytes)?;
    assert_eq!(metadata.wasm_sha256(), module.wasm_sha256());
    assert!(metadata.memory_len() > 0);
    drop(runtime);

    let restore_config = QuickJsHostConfig::new()
        .with_clock_time_ns(1_800_000_000_000_000_000)
        .with_timezone_offset_seconds(-7_200);

    let mut restored =
        module.restore_runtime_from_bytes_with_host_config(&snapshot_bytes, restore_config)?;

    assert_eq!(restored.eval_string("policyState")?, "snapshotted");
    assert_eq!(restored.eval_number("Date.now()")?, 1_800_000_000_000.0);
    assert_eq!(
        restored.eval_number("new Date(0).getTimezoneOffset()")?,
        120.0
    );

    println!(
        "host policy restored: state={} Date.now()={} timezone_offset_minutes={}",
        restored.eval_string("policyState")?,
        restored.eval_number("Date.now()")?,
        restored.eval_number("new Date(0).getTimezoneOffset()")?
    );

    Ok(())
}
