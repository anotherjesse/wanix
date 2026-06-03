use super::*;
use anyhow::anyhow;
use std::collections::HashMap;

#[test]
fn compile_bytecode_returns_module_bound_bytes() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let bytecode = vm.compile_bytecode("1 + 2")?;

    assert!(!bytecode.bytes().is_empty());
    assert_eq!(bytecode.wasm_sha256(), module.wasm_sha256());
    Ok(())
}

#[test]
fn eval_bytecode_value_round_trips_scalars() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let number = vm.compile_bytecode("1 + 2")?;
    assert_eq!(vm.eval_bytecode_value(&number)?, QuickJsValue::Number(3.0));

    let string = vm.compile_bytecode(r#""hello" + " " + "world""#)?;
    assert_eq!(
        vm.eval_bytecode_value(&string)?,
        QuickJsValue::String("hello world".into())
    );
    Ok(())
}

#[test]
fn bytecode_compile_does_not_execute_until_eval() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let bytecode = vm.compile_bytecode("globalThis.compiled = true; 42")?;
    assert_eq!(vm.eval_string("typeof globalThis.compiled")?, "undefined");

    assert_eq!(
        vm.eval_bytecode_value(&bytecode)?,
        QuickJsValue::Number(42.0)
    );
    assert_eq!(vm.eval_string("String(globalThis.compiled)")?, "true");
    Ok(())
}

#[test]
fn bytecode_transfers_between_runtimes_for_same_module() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut compiler = QuickJsRuntime::create(&engine, &module)?;
    let bytecode = compiler.compile_bytecode("40 + 2")?;
    drop(compiler);

    let mut evaluator = QuickJsRuntime::create(&engine, &module)?;
    assert_eq!(
        evaluator.eval_bytecode_value(&bytecode)?,
        QuickJsValue::Number(42.0)
    );
    Ok(())
}

#[test]
fn bytecode_parts_round_trip_for_persisted_storage() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let bytecode = vm.compile_bytecode("'persisted'")?;
    let (wasm_sha256, bytes) = bytecode.clone().into_parts();
    let restored = QuickJsBytecode::from_trusted_parts(wasm_sha256, &bytes)?;

    assert_eq!(restored.wasm_sha256(), module.wasm_sha256());
    assert_eq!(restored.bytes(), bytes);
    assert_eq!(
        vm.eval_bytecode_value(&restored)?,
        QuickJsValue::String("persisted".into())
    );
    Ok(())
}

#[test]
fn bytecode_rejects_empty_trusted_parts() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;

    let err = QuickJsBytecode::from_trusted_parts(module.wasm_sha256(), &[])
        .expect_err("empty bytecode should not be constructible");
    assert!(err.to_string().contains("must not be empty"));
    Ok(())
}

#[test]
fn bytecode_debug_hides_serialized_bytes() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let bytecode = vm.compile_bytecode(r#""secret-source-marker""#)?;

    let debug = format!("{bytecode:?}");
    assert!(debug.contains("byte_len"));
    assert!(debug.contains("wasm_sha256"));
    assert!(!debug.contains("secret-source-marker"));
    Ok(())
}

#[test]
fn restored_runtime_can_eval_trusted_bytecode() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let bytecode = vm.compile_bytecode("resumeMarker + 1")?;
    vm.eval_discard("globalThis.resumeMarker = 41")?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(
        restored.eval_bytecode_value(&bytecode)?,
        QuickJsValue::Number(42.0)
    );
    Ok(())
}

#[test]
fn module_bytecode_executes_module_side_effects() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let bytecode = vm.compile_bytecode_with_options(
        "globalThis.moduleBytecodeValue = 42; export const value = 42;",
        "bytecode-module.js",
        QuickJsBytecodeCompileOptions::new().as_module(),
    )?;
    vm.eval_bytecode_discard(&bytecode)?;

    assert_eq!(vm.eval_number("moduleBytecodeValue")?, 42.0);
    Ok(())
}

#[test]
fn module_bytecode_imports_source_from_reattached_loader_after_restore() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    vm.set_module_loader(source_map_loader(HashMap::from([(
        "answer.js",
        "export const answer = 42;",
    )])))?;

    let bytecode = vm.compile_bytecode_with_options(
        r#"
        import { answer } from "answer.js";
        globalThis.bytecodeImportedAnswer = answer;
        "#,
        "bytecode-main.js",
        QuickJsBytecodeCompileOptions::new().as_module(),
    )?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    let err = restored
        .eval_bytecode_discard(&bytecode)
        .expect_err("bytecode module imports should fail before loader reattachment");
    assert!(format!("{err:#}").contains("answer.js"));

    restored.set_module_loader(source_map_loader(HashMap::from([(
        "answer.js",
        "export const answer = 42;",
    )])))?;
    restored.eval_bytecode_discard(&bytecode)?;
    assert_eq!(restored.eval_number("bytecodeImportedAnswer")?, 42.0);
    Ok(())
}

#[test]
fn module_bytecode_preserves_normalized_import_names_after_restore() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    vm.set_module_loader_with_normalizer(
        |_base_name, specifier| match specifier {
            "./answer.js" => Ok("answer.js".to_string()),
            other => Ok(other.to_string()),
        },
        source_map_loader(HashMap::from([("answer.js", "export const answer = 42;")])),
    )?;

    let bytecode = vm.compile_bytecode_with_options(
        r#"
        import { answer } from "./answer.js";
        globalThis.normalizedBytecodeAnswer = answer;
        "#,
        "app/bytecode-main.js",
        QuickJsBytecodeCompileOptions::new().as_module(),
    )?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut loader_only = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    loader_only.set_module_loader(source_map_loader(HashMap::from([(
        "answer.js",
        "export const answer = 42;",
    )])))?;
    let err = loader_only
        .eval_bytecode_discard(&bytecode)
        .expect_err("module bytecode should still require restored normalizer policy");
    assert!(format!("{err:#}").contains("./answer.js"));

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    restored.set_module_loader_with_normalizer(
        |_base_name, specifier| match specifier {
            "./answer.js" => Ok("answer.js".to_string()),
            other => Ok(other.to_string()),
        },
        source_map_loader(HashMap::from([("answer.js", "export const answer = 42;")])),
    )?;
    restored.eval_bytecode_discard(&bytecode)?;

    assert_eq!(restored.eval_number("normalizedBytecodeAnswer")?, 42.0);
    Ok(())
}

#[test]
fn bytecode_compile_options_can_strip_serialized_metadata() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let source = "function hello() { return 'world'; } globalThis.hello = hello;";

    let full = vm.compile_bytecode(source)?;
    let stripped = vm.compile_bytecode_with_options(
        source,
        "<compile>",
        QuickJsBytecodeCompileOptions::new()
            .strip_source()
            .strip_debug(),
    )?;

    assert!(stripped.bytes().len() <= full.bytes().len());
    vm.eval_bytecode_discard(&stripped)?;
    assert_eq!(vm.eval_string("hello()")?, "world");
    Ok(())
}

#[test]
fn bytecode_compile_errors_surface_and_runtime_recovers() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .compile_bytecode("function {")
        .expect_err("invalid source should fail compilation");
    let message = format!("{err:#}");
    assert!(message.contains("bytecode compilation failed"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn repeated_bytecode_compile_keeps_quickjs_memory_accounting_bounded() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let payload = "x".repeat(64 * 1024);
    let source = format!("globalThis.largePayload = {payload:?}; largePayload.length");

    vm.set_memory_limit(512 * 1024)?;
    for _ in 0..2 {
        let bytecode = vm.compile_bytecode(&source)?;
        assert!(bytecode.bytes().len() > payload.len());
    }
    vm.clear_memory_limit()?;

    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn invalid_bytecode_errors_surface_and_runtime_recovers() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let bytecode = QuickJsBytecode::from_trusted_parts(module.wasm_sha256(), &[0xff])?;

    let err = vm
        .eval_bytecode_value(&bytecode)
        .expect_err("invalid bytecode should fail evaluation");
    assert!(format!("{err:#}").contains("QuickJS exception"));
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);
    Ok(())
}

#[test]
fn bytecode_rejects_wrong_module_identity_before_evaluation() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    let bytecode = vm.compile_bytecode("throw new Error('should not run')")?;

    let other_module = QuickJsModule::from_bytes(
        &engine,
        include_str!("../module_tests/minimal_abi.wat").as_bytes(),
    )?;
    let wrong_module_bytecode =
        QuickJsBytecode::from_trusted_parts(other_module.wasm_sha256(), bytecode.bytes())?;

    let err = vm
        .eval_bytecode_value(&wrong_module_bytecode)
        .expect_err("wrong module bytecode should fail before QuickJS evaluation");
    assert!(err.to_string().contains("bound to wasm module SHA-256"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn bytecode_rejects_invalid_filenames() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .compile_bytecode_with_options("1", "", QuickJsBytecodeCompileOptions::new())
        .expect_err("empty bytecode filenames should fail");
    assert!(err.to_string().contains("must not be empty"));

    let err = vm
        .compile_bytecode_with_options("1", "bad\0name.js", QuickJsBytecodeCompileOptions::new())
        .expect_err("NUL bytecode filenames should fail");
    assert!(err.to_string().contains("must not contain NUL"));
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
