use super::*;

#[test]
fn default_runtime_keeps_reference_intrinsics() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    assert_eq!(vm.eval_string("typeof Date")?, "function");
    assert_eq!(vm.eval_string("typeof eval")?, "function");
    assert_eq!(vm.eval_string("typeof Promise")?, "function");
    assert_eq!(vm.eval_string("typeof btoa")?, "function");
    Ok(())
}

#[test]
fn selected_intrinsics_disable_unwanted_builtins() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let intrinsics =
        QuickJsIntrinsics::EVAL | QuickJsIntrinsics::JSON | QuickJsIntrinsics::TYPED_ARRAYS;
    let mut vm = module.create_runtime_with_intrinsics(intrinsics)?;

    assert_eq!(vm.eval_string("typeof Date")?, "undefined");
    assert_eq!(vm.eval_string("typeof Promise")?, "undefined");
    assert_eq!(vm.eval_string("typeof btoa")?, "undefined");
    assert_eq!(vm.eval_string("typeof JSON")?, "object");
    assert_eq!(vm.eval_string("typeof Uint8Array")?, "function");
    Ok(())
}

#[test]
fn minimal_intrinsics_keep_base_objects_and_selected_features() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let intrinsics = QuickJsIntrinsics::EVAL | QuickJsIntrinsics::JSON | QuickJsIntrinsics::MAP_SET;
    let mut vm = module.create_runtime_with_intrinsics(intrinsics)?;

    assert_eq!(vm.eval_string("typeof Object")?, "function");
    assert_eq!(vm.eval_string("typeof Array")?, "function");
    assert_eq!(vm.eval_string("typeof JSON")?, "object");
    assert_eq!(vm.eval_string("typeof Map")?, "function");
    assert_eq!(vm.eval_string("typeof Date")?, "undefined");
    assert_eq!(vm.eval_string("typeof Promise")?, "undefined");
    assert_eq!(vm.eval_string("typeof Uint8Array")?, "undefined");
    Ok(())
}

#[test]
fn eval_free_runtime_can_execute_trusted_bytecode() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let mut compiler = module.create_runtime()?;
    let bytecode = compiler.compile_bytecode("globalThis.answer = 42; answer")?;
    let read_answer = compiler.compile_bytecode("globalThis.answer")?;
    drop(compiler);

    let intrinsics = QuickJsIntrinsics::JSON;
    let mut vm = module.create_runtime_with_intrinsics(intrinsics)?;

    let err = vm
        .eval_discard("globalThis.answer = 0")
        .expect_err("source eval should fail without the eval intrinsic");
    message_assert_contains(&err, "QuickJS exception");

    assert_eq!(
        vm.eval_bytecode_value(&bytecode)?,
        QuickJsValue::Number(42.0)
    );
    assert_eq!(
        vm.eval_bytecode_value(&read_answer)?,
        QuickJsValue::Number(42.0)
    );
    Ok(())
}

#[test]
fn intrinsics_are_part_of_snapshotted_context_state() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let intrinsics = QuickJsIntrinsics::EVAL | QuickJsIntrinsics::JSON;
    let mut vm = module.create_runtime_with_intrinsics(intrinsics)?;

    vm.eval_discard("globalThis.marker = JSON.stringify({ restored: true })")?;
    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(restored.eval_string("marker")?, r#"{"restored":true}"#);
    assert_eq!(restored.eval_string("typeof JSON")?, "object");
    assert_eq!(restored.eval_string("typeof Date")?, "undefined");
    Ok(())
}

#[test]
fn create_options_combine_host_config_and_intrinsics() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;
    let options = QuickJsCreateOptions::new()
        .with_host_config(QuickJsHostConfig::new().with_clock_time_ns(1_700_000_123_000_000_000))
        .with_intrinsics(QuickJsIntrinsics::EVAL | QuickJsIntrinsics::DATE);
    let mut vm = module.create_runtime_with_options(options)?;

    assert_eq!(vm.eval_number("Date.now()")?, 1_700_000_123_000.0);
    assert_eq!(vm.eval_string("typeof Promise")?, "undefined");
    Ok(())
}

#[test]
fn intrinsic_masks_preserve_raw_bits() {
    let mut custom = QuickJsIntrinsics::from_bits(0x8000_0001);

    assert_eq!(custom.bits(), 0x8000_0001);
    assert!(custom.contains(QuickJsIntrinsics::DATE));
    custom |= QuickJsIntrinsics::EVAL;
    assert!(custom.contains(QuickJsIntrinsics::DATE | QuickJsIntrinsics::EVAL));
    assert_eq!(
        (QuickJsIntrinsics::ALL & !QuickJsIntrinsics::EVAL).bits(),
        0xffff_fffd
    );
    assert_eq!(custom.without(QuickJsIntrinsics::DATE).bits(), 0x8000_0002);
}

fn message_assert_contains(err: &anyhow::Error, expected: &str) {
    let message = format!("{err:#}");
    assert!(
        message.contains(expected),
        "expected error to contain {expected:?}, got {message:?}"
    );
}
