use anyhow::{Result, anyhow};
use rust_wasi_quickjs::QuickJsModule;
use std::collections::HashMap;
use wasmtime::Engine;

fn main() -> Result<()> {
    let engine = Engine::default();
    let module = QuickJsModule::from_file(&engine, "fixtures/quickjs.wasm")?;
    let mut runtime = module.create_runtime()?;

    runtime.set_module_loader(source_map_loader(HashMap::from([(
        "initial.js",
        "export const value = 42;",
    )])))?;
    runtime.eval_module_discard(
        r#"
        import { value } from "initial.js";
        globalThis.initialModuleValue = value;
        "#,
        "before-snapshot.js",
    )?;

    let snapshot_bytes = runtime.snapshot()?.try_to_bytes()?;
    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;

    assert_eq!(restored.eval_number("initialModuleValue")?, 42.0);
    let err = restored
        .eval_module_discard(
            r#"
            import { value } from "later-before-loader.js";
            globalThis.unreachable = value;
            "#,
            "future-before-loader.js",
        )
        .expect_err("future imports need the Rust loader to be reattached");
    assert!(format!("{err:#}").contains("later-before-loader.js"));

    restored.set_module_loader(source_map_loader(HashMap::from([(
        "later-after-loader.js",
        "export const value = 7 * 6;",
    )])))?;
    restored.eval_module_discard(
        r#"
        import { value } from "later-after-loader.js";
        globalThis.laterModuleValue = value;
        "#,
        "future-after-loader.js",
    )?;

    println!(
        "module loader restored: initial={} later={}",
        restored.eval_number("initialModuleValue")?,
        restored.eval_number("laterModuleValue")?
    );
    Ok(())
}

fn source_map_loader(
    sources: HashMap<&'static str, &'static str>,
) -> impl FnMut(&str) -> Result<String> {
    let sources: HashMap<String, String> = sources
        .into_iter()
        .map(|(name, source)| (name.to_string(), source.to_string()))
        .collect();
    move |name| {
        sources
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("missing module {name}"))
    }
}
