use super::*;

#[test]
fn restores_global_state_from_snapshot() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;

    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    assert_eq!(vm.eval_number("globalThis.counter = 42")?, 42.0);

    let snapshot = vm.snapshot()?;
    drop(vm);

    let mut restored = QuickJsRuntime::restore(&engine, &module, &snapshot)?;
    assert_eq!(restored.eval_number("counter")?, 42.0);
    Ok(())
}

#[test]
fn serializes_snapshot_bytes_and_restores() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;

    let mut vm = QuickJsRuntime::create(&engine, &module)?;
    vm.eval_discard(
        r#"
        globalThis.serializedLabel = "still here";
        globalThis.serializedNumber = 9001;
        "#,
    )?;

    let snapshot = vm.snapshot()?;
    let bytes = snapshot.try_to_bytes()?;

    let decoded = Snapshot::from_bytes_for_module(&bytes, &module)?;
    assert_eq!(decoded, snapshot);
    drop(vm);

    let mut restored = QuickJsRuntime::restore(&engine, &module, &decoded)?;
    assert_eq!(restored.eval_string("serializedLabel")?, "still here");
    assert_eq!(restored.eval_number("serializedNumber")?, 9001.0);
    Ok(())
}

#[test]
fn module_owned_helpers_create_and_restore_from_bytes() -> Result<()> {
    let (_engine, module) = quickjs_fixture()?;

    let mut vm = module.create_runtime()?;
    vm.eval_discard("globalThis.message = 'module owned restore'")?;
    let bytes = vm.snapshot()?.try_to_bytes()?;

    let mut restored = module.restore_runtime_from_bytes(&bytes)?;

    assert_eq!(restored.eval_string("message")?, "module owned restore");
    Ok(())
}
