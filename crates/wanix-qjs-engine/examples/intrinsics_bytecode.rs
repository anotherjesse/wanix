use anyhow::Result;
use rust_wasi_quickjs::{
    QuickJsBytecodeCompileOptions, QuickJsIntrinsics, QuickJsModule, QuickJsValue,
};
use std::path::PathBuf;
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;

    let mut compiler = module.create_runtime()?;
    let bytecode = compiler.compile_bytecode_with_options(
        "globalThis.bytecodeAnswer = 42; bytecodeAnswer",
        "trusted-bytecode.js",
        QuickJsBytecodeCompileOptions::new()
            .strip_source()
            .strip_debug(),
    )?;
    let read_answer = compiler.compile_bytecode("globalThis.bytecodeAnswer")?;
    let read_date_type = compiler.compile_bytecode("typeof Date")?;
    drop(compiler);

    let intrinsics = QuickJsIntrinsics::DATE | QuickJsIntrinsics::JSON;
    let mut runtime = module.create_runtime_with_intrinsics(intrinsics)?;

    let err = runtime
        .eval_discard("globalThis.sourceEvalAnswer = 0")
        .expect_err("source eval should fail when the eval intrinsic is disabled");
    assert!(format!("{err:#}").contains("QuickJS exception"));
    assert_eq!(
        runtime.eval_bytecode_value(&bytecode)?,
        QuickJsValue::Number(42.0)
    );

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    drop(runtime);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(
        restored.eval_bytecode_value(&read_date_type)?,
        QuickJsValue::String("function".into())
    );
    let answer = match restored.eval_bytecode_value(&read_answer)? {
        QuickJsValue::Number(answer) => answer,
        other => anyhow::bail!("unexpected bytecode answer after restore: {other:?}"),
    };
    assert_eq!(answer, 42.0);

    println!(
        "intrinsics bytecode restored: bytecode={} bytes answer={}",
        bytecode.bytes().len(),
        answer
    );
    Ok(())
}
