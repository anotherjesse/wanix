use anyhow::{Result, bail};
use rust_wasi_quickjs::{QuickJsHostValue, QuickJsModule};
use std::path::PathBuf;
use wasmtime::Engine;

fn first_number_arg(args: &[QuickJsHostValue], name: &str) -> Result<f64> {
    match args {
        [QuickJsHostValue::Number(value)] => Ok(*value),
        _ => bail!("{name} expects one number"),
    }
}

fn main() -> Result<()> {
    let engine = Engine::default();
    let quickjs_wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/quickjs.wasm");
    let module = QuickJsModule::from_file(&engine, quickjs_wasm)?;

    let mut runtime = module.create_runtime()?;
    runtime.define_global_host_function("hostOffset", |args| {
        Ok(QuickJsHostValue::Number(
            first_number_arg(args, "hostOffset")? + 1.0,
        ))
    })?;

    assert_eq!(runtime.eval_number("hostOffset(41)")?, 42.0);

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    drop(runtime);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    restored.register_host_callback("hostOffset", |args| {
        Ok(QuickJsHostValue::Number(
            first_number_arg(args, "hostOffset")? + 100.0,
        ))
    })?;

    assert_eq!(restored.eval_number("hostOffset(23)")?, 123.0);

    println!(
        "host callback restored by stable name: hostOffset(23)={}",
        restored.eval_number("hostOffset(23)")?
    );

    Ok(())
}
