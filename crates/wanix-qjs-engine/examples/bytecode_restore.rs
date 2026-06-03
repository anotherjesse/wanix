use anyhow::Result;
use rust_wasi_quickjs::{QuickJsBytecodeCompileOptions, QuickJsModule, QuickJsValue};
use std::path::PathBuf;
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;

    let mut runtime = module.create_runtime()?;
    let bytecode = runtime.compile_bytecode_with_options(
        "globalThis.bytecodeRan = true; resumeMarker + 1",
        "bytecode-restore.js",
        QuickJsBytecodeCompileOptions::new()
            .strip_source()
            .strip_debug(),
    )?;
    runtime.eval_discard("globalThis.resumeMarker = 41")?;

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    drop(runtime);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(
        restored.eval_bytecode_value(&bytecode)?,
        QuickJsValue::Number(42.0)
    );
    assert_eq!(restored.eval_string("String(bytecodeRan)")?, "true");

    println!(
        "bytecode restored: {} bytes, bytecodeRan={}",
        bytecode.bytes().len(),
        restored.eval_string("String(bytecodeRan)")?
    );
    Ok(())
}
