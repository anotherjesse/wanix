use super::*;

#[test]
fn eval_value_returns_copied_scalars() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let cases = [
        ("undefined", QuickJsValue::Undefined),
        ("null", QuickJsValue::Null),
        ("true", QuickJsValue::Bool(true)),
        ("false", QuickJsValue::Bool(false)),
        ("21 * 2", QuickJsValue::Number(42.0)),
        (
            "'hello from js'",
            QuickJsValue::String("hello from js".into()),
        ),
        ("42n", QuickJsValue::BigIntI64(42)),
        ("-1n", QuickJsValue::BigIntI64(-1)),
        ("4294967296n", QuickJsValue::BigIntI64(1_i64 << 32)),
        ("9223372036854775807n", QuickJsValue::BigIntI64(i64::MAX)),
        (
            "(-9223372036854775807n - 1n)",
            QuickJsValue::BigIntI64(i64::MIN),
        ),
    ];

    for (code, expected) in cases {
        assert_eq!(vm.eval_value(code)?, expected);
    }

    let err = vm
        .eval_value("({ answer: 42 })")
        .expect_err("object results should stay out of the copied scalar API");
    assert!(err.to_string().contains("not a supported copied scalar"));
    Ok(())
}

#[test]
fn set_and_get_global_value_round_trips_scalars() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let values = [
        ("scalarUndefined", QuickJsValue::Undefined),
        ("scalarNull", QuickJsValue::Null),
        ("scalarTrue", QuickJsValue::Bool(true)),
        ("scalarFalse", QuickJsValue::Bool(false)),
        ("scalarNumber", QuickJsValue::Number(9001.0)),
        ("scalarString", QuickJsValue::String("round trip".into())),
        ("scalarBigInt", QuickJsValue::BigIntI64(i64::MIN)),
    ];

    for (name, value) in values {
        vm.set_global_value(name, value.clone())?;
        assert_eq!(vm.get_global_value(name)?, value);
    }

    assert_eq!(vm.eval_string("typeof scalarUndefined")?, "undefined");
    assert_eq!(vm.eval_string("scalarNull === null ? 'yes' : 'no'")?, "yes");
    assert_eq!(vm.eval_number("scalarNumber + 1")?, 9002.0);
    assert_eq!(
        vm.eval_string("scalarBigInt === -(2n ** 63n) ? 'yes' : 'no'")?,
        "yes"
    );
    Ok(())
}

#[test]
fn global_scalar_value_names_reject_embedded_nul_bytes() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_global_value("truncated", QuickJsValue::Number(1.0))?;

    let err = vm
        .set_global_value("truncated\0suffix", QuickJsValue::Number(2.0))
        .expect_err("embedded NULs should not silently truncate global writes");
    assert!(err.to_string().contains("must not contain NUL"));
    assert_eq!(vm.get_global_value("truncated")?, QuickJsValue::Number(1.0));

    let err = vm
        .get_global_value("truncated\0suffix")
        .expect_err("embedded NULs should not silently truncate global reads");
    assert!(err.to_string().contains("must not contain NUL"));

    let err = vm
        .call_global_function("truncated\0suffix", &[])
        .expect_err("embedded NULs should not silently truncate global calls");
    assert!(err.to_string().contains("must not contain NUL"));
    Ok(())
}

#[test]
fn call_global_function_accepts_scalar_args_and_returns_scalar() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard(
        r#"
        globalThis.describeScalars = (u, n, b, x, s) =>
          [typeof u, n === null, b, x + 1, s].join("|");
        globalThis.returnNull = () => null;
        globalThis.returnUndefined = () => undefined;
        globalThis.returnBool = value => value === true;
        globalThis.bumpBigInt = value => value + 2n;
        "#,
    )?;

    let description = vm.call_global_function(
        "describeScalars",
        &[
            QuickJsValue::Undefined,
            QuickJsValue::Null,
            QuickJsValue::Bool(true),
            QuickJsValue::Number(41.0),
            QuickJsValue::String("payload".into()),
        ],
    )?;
    assert_eq!(
        description,
        QuickJsValue::String("undefined|true|true|42|payload".into())
    );

    assert_eq!(
        vm.call_global_function("returnNull", &[])?,
        QuickJsValue::Null
    );
    assert_eq!(
        vm.call_global_function("returnUndefined", &[])?,
        QuickJsValue::Undefined
    );
    assert_eq!(
        vm.call_global_function("returnBool", &[QuickJsValue::Bool(true)])?,
        QuickJsValue::Bool(true)
    );
    assert_eq!(
        vm.call_global_function("bumpBigInt", &[QuickJsValue::BigIntI64(i64::MAX - 2)])?,
        QuickJsValue::BigIntI64(i64::MAX)
    );
    Ok(())
}

#[test]
fn call_global_function_rejects_unsupported_return_values() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard("globalThis.returnObject = () => ({ answer: 42 })")?;
    let err = vm
        .call_global_function("returnObject", &[])
        .expect_err("object returns should stay out of the copied scalar API");
    assert!(err.to_string().contains("not a supported copied scalar"));
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);
    Ok(())
}

#[test]
fn copied_bigints_reject_values_outside_i64_range() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    for expression in ["2n ** 63n", "-(2n ** 63n) - 1n"] {
        let err = vm
            .eval_value(expression)
            .expect_err("copied BigInts outside i64 should fail exactly");
        let message = format!("{err:#}");
        assert!(message.contains("qjs_get_big_int64 failed"));
        assert!(message.contains("outside signed 64-bit range"));
        assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    }
    Ok(())
}

#[test]
fn restored_runtime_preserves_scalar_globals_and_calls() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_global_value("answer", QuickJsValue::Number(42.0))?;
    vm.set_global_value("label", QuickJsValue::String("before snapshot".into()))?;
    vm.set_global_value("big", QuickJsValue::BigIntI64(1_i64 << 40))?;
    vm.eval_discard(
        r#"
        globalThis.describeAfterRestore = (value, include) =>
          include ? `${label}:${value + answer}` : null;
        globalThis.bumpBigAfterRestore = value => value + big;
        "#,
    )?;

    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    assert_eq!(
        restored.get_global_value("label")?,
        QuickJsValue::String("before snapshot".into())
    );
    assert_eq!(
        restored.call_global_function(
            "describeAfterRestore",
            &[QuickJsValue::Number(8.0), QuickJsValue::Bool(true)]
        )?,
        QuickJsValue::String("before snapshot:50".into())
    );
    assert_eq!(
        restored.call_global_function("bumpBigAfterRestore", &[QuickJsValue::BigIntI64(2)])?,
        QuickJsValue::BigIntI64((1_i64 << 40) + 2)
    );
    assert_eq!(
        restored.call_global_function(
            "describeAfterRestore",
            &[QuickJsValue::Number(8.0), QuickJsValue::Bool(false)]
        )?,
        QuickJsValue::Null
    );
    Ok(())
}
