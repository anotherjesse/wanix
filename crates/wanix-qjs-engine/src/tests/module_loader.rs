use super::*;
use anyhow::{Result, anyhow};
use std::collections::HashMap;

#[test]
fn module_loader_imports_source_from_rust() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader(source_map_loader(HashMap::from([(
        "math.js",
        "export const answer = 42;",
    )])))?;
    vm.eval_module_discard(
        r#"
        import { answer } from "math.js";
        globalThis.moduleAnswer = answer;
        "#,
        "main.js",
    )?;

    assert_eq!(vm.eval_number("moduleAnswer")?, 42.0);
    Ok(())
}

#[test]
fn module_loader_uses_optional_normalizer() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader_with_normalizer(
        |_base_name, specifier| match specifier {
            "./math.js" => Ok("math.js".to_string()),
            other => Ok(other.to_string()),
        },
        source_map_loader(HashMap::from([(
            "math.js",
            "export const doubled = 21 * 2;",
        )])),
    )?;
    vm.eval_module_discard(
        r#"
        import { doubled } from "./math.js";
        globalThis.normalizedAnswer = doubled;
        "#,
        "app/main.js",
    )?;

    assert_eq!(vm.eval_number("normalizedAnswer")?, 42.0);
    Ok(())
}

#[test]
fn module_loader_resolves_transitive_imports() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader(source_map_loader(HashMap::from([
        (
            "a.js",
            "import { b } from \"b.js\"; export const a = b + 1;",
        ),
        ("b.js", "export const b = 41;"),
    ])))?;
    vm.eval_module_discard(
        r#"
        import { a } from "a.js";
        globalThis.transitiveAnswer = a;
        "#,
        "transitive-main.js",
    )?;

    assert_eq!(vm.eval_number("transitiveAnswer")?, 42.0);
    Ok(())
}

#[test]
fn missing_module_error_recovers_runtime() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader(source_map_loader(HashMap::new()))?;
    let err = vm
        .eval_module_discard(
            r#"
            import { missing } from "missing.js";
            globalThis.missing = missing;
            "#,
            "missing-main.js",
        )
        .expect_err("missing module should fail");
    let message = format!("{err:#}");
    assert!(message.contains("could not load module"));
    assert!(message.contains("missing.js"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn module_loader_panics_are_guest_catchable() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader(|_name| -> Result<String> {
        panic!("panic from Rust module loader");
    })?;

    let err = vm
        .eval_module_discard(
            r#"
            import { value } from "panic.js";
            globalThis.value = value;
            "#,
            "panic-main.js",
        )
        .expect_err("loader panic should surface as a QuickJS exception");
    let message = format!("{err:#}");
    assert!(message.contains("could not load module"));
    assert!(message.contains("panic.js"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn restored_runtime_requires_loader_reattachment_for_future_imports() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader(source_map_loader(HashMap::from([(
        "initial.js",
        "export const initial = 42;",
    )])))?;
    vm.eval_module_discard(
        r#"
        import { initial } from "initial.js";
        globalThis.initialModuleValue = initial;
        "#,
        "before-snapshot.js",
    )?;
    assert_eq!(vm.eval_number("initialModuleValue")?, 42.0);

    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(restored.eval_number("initialModuleValue")?, 42.0);

    let err = restored
        .eval_module_discard(
            r#"
            import { later } from "later-before-loader.js";
            globalThis.laterBeforeLoader = later;
            "#,
            "future-before-loader.js",
        )
        .expect_err("future imports should fail until the Rust loader is reattached");
    let message = format!("{err:#}");
    assert!(message.contains("later-before-loader.js"));

    restored.set_module_loader(source_map_loader(HashMap::from([(
        "later-after-loader.js",
        "export const later = 100;",
    )])))?;
    restored.eval_module_discard(
        r#"
        import { later } from "later-after-loader.js";
        globalThis.laterAfterLoader = later;
        "#,
        "future-after-loader.js",
    )?;

    assert_eq!(restored.eval_number("laterAfterLoader")?, 100.0);
    Ok(())
}

#[test]
fn restored_runtime_preserves_already_loaded_module_state_without_loader() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader(source_map_loader(HashMap::from([(
        "counter.js",
        r#"
        export let count = 1;
        export function bump() {
          count += 1;
          return count;
        }
        "#,
    )])))?;
    vm.eval_module_discard(
        r#"
        import { bump } from "counter.js";
        globalThis.beforeSnapshotBump = bump();
        "#,
        "counter-main-before.js",
    )?;
    assert_eq!(vm.eval_number("beforeSnapshotBump")?, 2.0);

    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    restored.set_module_loader(|name| anyhow::bail!("unexpected reload of {name}"))?;
    restored.eval_module_discard(
        r#"
        import { bump } from "counter.js";
        globalThis.afterSnapshotBump = bump();
        "#,
        "counter-main-after.js",
    )?;

    assert_eq!(restored.eval_number("afterSnapshotBump")?, 3.0);
    Ok(())
}

#[test]
fn module_eval_rejects_invalid_filenames() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .eval_module_discard("globalThis.never = true;", "")
        .expect_err("empty module filenames should fail");
    assert!(err.to_string().contains("must not be empty"));

    let err = vm
        .eval_module_discard("globalThis.never = true;", "bad\0name.js")
        .expect_err("NUL module filenames should fail");
    assert!(err.to_string().contains("must not contain NUL"));
    Ok(())
}

#[test]
fn module_normalizer_rejects_nul_names_and_runtime_recovers() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_module_loader_with_normalizer(
        |_base_name, _specifier| Ok("bad\0name.js".to_string()),
        |_name| Ok("export const value = 1;".to_string()),
    )?;

    let err = vm
        .eval_module_discard(
            r#"
            import { value } from "./bad.js";
            globalThis.value = value;
            "#,
            "nul-normalizer-main.js",
        )
        .expect_err("NUL normalized module names should fail");
    let message = format!("{err:#}");
    assert!(message.contains("could not normalize module"));
    assert!(message.contains("./bad.js"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
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
