use anyhow::Result;
use rust_wasi_quickjs::{QuickJsModule, QuickJsValue};
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let module = QuickJsModule::from_file(&engine, "fixtures/quickjs.wasm")?;
    let mut runtime = module.create_runtime()?;

    runtime.set_global_value("label", QuickJsValue::String("snapshotted".into()))?;
    runtime.set_global_value("offset", QuickJsValue::Number(40.0))?;
    runtime.eval_discard(
        r#"
        globalThis.describeScalar = (name, extra, include) =>
          include ? `${label}:${name}:${offset + extra}` : null;
        "#,
    )?;

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;

    assert_eq!(
        restored.get_global_value("label")?,
        QuickJsValue::String("snapshotted".into())
    );

    let description = restored.call_global_function(
        "describeScalar",
        &[
            QuickJsValue::String("resumed".into()),
            QuickJsValue::Number(2.0),
            QuickJsValue::Bool(true),
        ],
    )?;
    assert_eq!(
        description,
        QuickJsValue::String("snapshotted:resumed:42".into())
    );
    assert_eq!(
        restored.call_global_function(
            "describeScalar",
            &[
                QuickJsValue::String("hidden".into()),
                QuickJsValue::Number(2.0),
                QuickJsValue::Bool(false),
            ],
        )?,
        QuickJsValue::Null
    );

    println!("scalar values restored: {description:?}");
    Ok(())
}
