use super::*;
use anyhow::{anyhow, bail};

#[test]
fn global_host_function_handles_numbers_strings_and_mutable_state() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostAdd", |args| match args {
        [
            QuickJsHostValue::Number(left),
            QuickJsHostValue::Number(right),
        ] => Ok(QuickJsHostValue::Number(left + right)),
        _ => bail!("hostAdd expects two numbers"),
    })?;
    vm.define_global_host_function("hostEcho", |args| match args {
        [QuickJsHostValue::String(value)] => Ok(QuickJsHostValue::String(format!("echo: {value}"))),
        _ => bail!("hostEcho expects one string"),
    })?;
    vm.define_global_host_function("hostUndefined", |args| match args {
        [QuickJsHostValue::Undefined] => Ok(QuickJsHostValue::Undefined),
        _ => bail!("hostUndefined expects undefined"),
    })?;

    let mut count = 0.0;
    vm.define_global_host_function("hostNext", move |_args| {
        count += 1.0;
        Ok(QuickJsHostValue::Number(count))
    })?;

    assert_eq!(vm.eval_number("hostAdd(10, 20)")?, 30.0);
    assert_eq!(vm.eval_string("hostEcho('payload')")?, "echo: payload");
    assert_eq!(
        vm.eval_string("typeof hostUndefined(undefined)")?,
        "undefined"
    );
    assert_eq!(vm.eval_number("hostNext() + hostNext()")?, 3.0);
    Ok(())
}

#[test]
fn duplicate_host_callback_names_are_rejected() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.register_host_callback("dupe", |_args| Ok(QuickJsHostValue::Undefined))?;
    let err = vm
        .register_host_callback("dupe", |_args| Ok(QuickJsHostValue::Undefined))
        .expect_err("duplicate callback registration should fail");
    assert!(err.to_string().contains("already registered"));

    let err = vm
        .define_global_host_function("dupe", |_args| Ok(QuickJsHostValue::Undefined))
        .expect_err("duplicate define should fail before replacing the global");
    assert!(err.to_string().contains("already registered"));
    Ok(())
}

#[test]
fn invalid_host_callback_names_are_rejected() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .register_host_callback("", |_args| Ok(QuickJsHostValue::Undefined))
        .expect_err("empty callback names should fail");
    assert!(err.to_string().contains("must not be empty"));

    let err = vm
        .register_host_callback("bad\0name", |_args| Ok(QuickJsHostValue::Undefined))
        .expect_err("NUL callback names should fail");
    assert!(err.to_string().contains("must not contain NUL"));
    Ok(())
}

#[test]
fn host_callback_errors_are_guest_catchable() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostBoom", |_args| Err(anyhow!("boom from Rust")))?;

    let message = vm.eval_string(
        r#"
        try {
          hostBoom("payload");
          "not reached";
        } catch (err) {
          String(err);
        }
        "#,
    )?;

    assert!(message.contains("Rust host callback failed"));
    assert!(message.contains("boom from Rust"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn host_callback_panics_are_guest_catchable() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostPanic", |_args| -> Result<QuickJsHostValue> {
        panic!("panic from Rust callback");
    })?;

    let message = vm.eval_string(
        r#"
        try {
          hostPanic();
          "not reached";
        } catch (err) {
          String(err);
        }
        "#,
    )?;

    assert!(message.contains("Rust host callback failed"));
    assert!(message.contains("host callback panicked"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);
    Ok(())
}

#[test]
fn unsupported_host_callback_argument_types_throw() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostAccept", |_args| Ok(QuickJsHostValue::Undefined))?;

    for expression in ["{}", "[]", "(() => 1)"] {
        let message = vm.eval_string(&format!(
            r#"
            try {{
              hostAccept({expression});
              "not reached";
            }} catch (err) {{
              String(err);
            }}
            "#
        ))?;
        assert!(message.contains("unsupported host callback argument type"));
    }
    Ok(())
}

#[test]
fn scalar_host_callbacks_reject_binary_arguments() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostAccept", |_args| Ok(QuickJsHostValue::Undefined))?;

    for expression in [
        "new ArrayBuffer(2)",
        "new Uint8Array([1, 2, 3])",
        "new Int16Array([1, 2])",
        "new DataView(new Uint8Array([1, 2, 3]).buffer)",
    ] {
        let message = vm.eval_string(&format!(
            r#"
            try {{
              hostAccept({expression});
              "not reached";
            }} catch (err) {{
              String(err);
            }}
            "#
        ))?;
        assert!(message.contains("unsupported host callback argument type"));
    }
    Ok(())
}

#[test]
fn binary_host_callbacks_accept_and_return_copied_bytes() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function_with_binary_values("hostBytes", |args| match args {
        [
            QuickJsCallbackValue::Binary(QuickJsBinaryValue::ArrayBuffer(buffer)),
            QuickJsCallbackValue::Binary(QuickJsBinaryValue::Uint8Array(view)),
        ] => {
            let mut bytes = buffer.clone();
            bytes.extend(view.iter().rev());
            Ok(QuickJsBinaryValue::uint8_array(&bytes)?.into())
        }
        _ => bail!("hostBytes got unexpected arguments: {args:?}"),
    })?;

    let result = vm.eval_binary_value(
        r#"
        hostBytes(
          new Uint8Array([1, 2, 3]).buffer,
          new Uint8Array([9, 8, 7, 6]).subarray(1, 3)
        )
        "#,
    )?;

    assert_eq!(result, QuickJsBinaryValue::Uint8Array(vec![1, 2, 3, 7, 8]));
    Ok(())
}

#[test]
fn binary_host_callbacks_accept_and_return_typed_arrays() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function_with_binary_values("hostTyped", |args| match args {
        [
            QuickJsCallbackValue::Binary(QuickJsBinaryValue::TypedArray {
                kind: QuickJsTypedArrayKind::Int16,
                bytes,
            }),
        ] => Ok(QuickJsBinaryValue::typed_array(
            QuickJsTypedArrayKind::Uint8Clamped,
            &[bytes[1], bytes[0], bytes[3], bytes[2]],
        )?
        .into()),
        _ => bail!("hostTyped got unexpected arguments: {args:?}"),
    })?;

    let result = vm.eval_binary_value(
        r#"
        const buffer = new ArrayBuffer(8);
        new Uint8Array(buffer).set([1, 2, 3, 4, 5, 6, 7, 8]);
        hostTyped(new Int16Array(buffer, 2, 2))
        "#,
    )?;

    assert_eq!(
        result,
        QuickJsBinaryValue::TypedArray {
            kind: QuickJsTypedArrayKind::Uint8Clamped,
            bytes: vec![4, 3, 6, 5],
        }
    );
    Ok(())
}

#[test]
fn binary_host_callbacks_accept_and_return_data_views() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function_with_binary_values("hostView", |args| match args {
        [QuickJsCallbackValue::Binary(QuickJsBinaryValue::DataView(bytes))] => {
            Ok(QuickJsBinaryValue::data_view(&[bytes[2], bytes[1]])?.into())
        }
        _ => bail!("hostView got unexpected arguments: {args:?}"),
    })?;

    let result = vm.eval_binary_value(
        r#"
        const buffer = new ArrayBuffer(6);
        new Uint8Array(buffer).set([1, 2, 3, 4, 5, 6]);
        hostView(new DataView(buffer, 2, 3))
        "#,
    )?;

    assert_eq!(result, QuickJsBinaryValue::DataView(vec![5, 4]));
    assert_eq!(
        vm.eval_string("hostView(new DataView(new Uint8Array([9, 8, 7]).buffer)) instanceof DataView ? 'yes' : 'no'")?,
        "yes"
    );
    Ok(())
}

#[test]
fn binary_host_callbacks_can_return_array_buffers() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function_with_binary_values("hostBuffer", |args| match args {
        [QuickJsCallbackValue::Scalar(QuickJsValue::Number(value))] => {
            Ok(QuickJsBinaryValue::array_buffer(&[*value as u8, 42])?.into())
        }
        _ => bail!("hostBuffer expects one number"),
    })?;

    assert_eq!(
        vm.eval_binary_value("hostBuffer(7)")?,
        QuickJsBinaryValue::ArrayBuffer(vec![7, 42])
    );
    Ok(())
}

#[test]
fn host_callbacks_accept_and_return_null_and_bool_scalars() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostDescribeBooleans", |args| match args {
        [
            QuickJsHostValue::Undefined,
            QuickJsHostValue::Null,
            QuickJsHostValue::Bool(true),
            QuickJsHostValue::Bool(false),
        ] => Ok(QuickJsHostValue::String("undefined|null|true|false".into())),
        _ => bail!("hostDescribeBooleans got unexpected arguments: {args:?}"),
    })?;
    vm.define_global_host_function("hostReturnNull", |args| match args {
        [QuickJsHostValue::Bool(true)] => Ok(QuickJsHostValue::Null),
        [QuickJsHostValue::Bool(false)] => Ok(QuickJsHostValue::Bool(false)),
        _ => bail!("hostReturnNull expects one bool"),
    })?;

    assert_eq!(
        vm.eval_string("hostDescribeBooleans(undefined, null, true, false)")?,
        "undefined|null|true|false"
    );
    assert_eq!(
        vm.eval_string("hostReturnNull(true) === null ? 'null' : 'nope'")?,
        "null"
    );
    assert_eq!(
        vm.eval_string("hostReturnNull(false) === false ? 'false' : 'nope'")?,
        "false"
    );
    Ok(())
}

#[test]
fn host_callbacks_accept_and_return_bigint_scalars() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostDescribeBigInts", |args| match args {
        [
            QuickJsHostValue::BigIntI64(max),
            QuickJsHostValue::BigIntI64(min),
        ] => Ok(QuickJsHostValue::String(format!("{max}:{min}"))),
        _ => bail!("hostDescribeBigInts got unexpected arguments: {args:?}"),
    })?;
    vm.define_global_host_function("hostBumpBigInt", |args| match args {
        [QuickJsHostValue::BigIntI64(value)] => Ok(QuickJsHostValue::BigIntI64(value + 5)),
        _ => bail!("hostBumpBigInt expects one BigInt"),
    })?;

    assert_eq!(
        vm.eval_string("hostDescribeBigInts(9223372036854775807n, -9223372036854775808n)")?,
        "9223372036854775807:-9223372036854775808"
    );
    assert_eq!(
        vm.eval_value("hostBumpBigInt(37n)")?,
        QuickJsHostValue::BigIntI64(42)
    );
    Ok(())
}

#[test]
fn binary_host_callbacks_treat_bigints_as_scalar_values() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function_with_binary_values("hostMixedBigInt", |args| match args {
        [
            QuickJsCallbackValue::Scalar(QuickJsValue::BigIntI64(value)),
            QuickJsCallbackValue::Binary(QuickJsBinaryValue::Uint8Array(bytes)),
        ] => Ok(QuickJsValue::String(format!("{value}:{}", bytes[0])).into()),
        _ => bail!("hostMixedBigInt got unexpected arguments: {args:?}"),
    })?;
    vm.define_global_host_function_with_binary_values("hostReturnBigInt", |_args| {
        Ok(QuickJsValue::BigIntI64(-42).into())
    })?;

    assert_eq!(
        vm.eval_string("hostMixedBigInt(17n, new Uint8Array([9]))")?,
        "17:9"
    );
    assert_eq!(
        vm.eval_value("hostReturnBigInt(new Uint8Array([1, 2, 3]))")?,
        QuickJsValue::BigIntI64(-42)
    );
    Ok(())
}

#[test]
fn restored_binary_host_function_requires_binary_restore_registration() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function_with_binary_values("hostBump", |_args| {
        Ok(QuickJsBinaryValue::array_buffer(&[1])?.into())
    })?;
    assert_eq!(
        vm.eval_binary_value("hostBump(new Uint8Array([0]))")?,
        QuickJsBinaryValue::ArrayBuffer(vec![1])
    );

    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    let err = restored
        .eval_binary_value("hostBump(new Uint8Array([41]))")
        .expect_err("restored host function should fail until Rust callback is reattached");
    let message = err.to_string();
    assert!(message.contains("QuickJS exception"));
    assert!(message.contains("host callback 'hostBump' is not registered"));

    restored.register_host_callback_with_binary_values("hostBump", |args| match args {
        [QuickJsCallbackValue::Binary(QuickJsBinaryValue::Uint8Array(bytes))] => {
            Ok(QuickJsBinaryValue::array_buffer(&[bytes[0] + 1])?.into())
        }
        _ => bail!("hostBump expects one Uint8Array"),
    })?;
    assert_eq!(
        restored.eval_binary_value("hostBump(new Uint8Array([41]))")?,
        QuickJsBinaryValue::ArrayBuffer(vec![42])
    );
    Ok(())
}

#[test]
fn restored_host_function_requires_restore_time_registration() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostTag", |_args| {
        Ok(QuickJsHostValue::String("before snapshot".to_string()))
    })?;
    assert_eq!(vm.eval_string("hostTag()")?, "before snapshot");

    let snapshot_bytes = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot_bytes)?;
    let err = restored
        .eval_string("hostTag()")
        .expect_err("restored host function should fail until Rust callback is reattached");
    let message = err.to_string();
    assert!(message.contains("Rust host callback failed"));
    assert!(message.contains("host callback 'hostTag' is not registered"));

    restored.register_host_callback("hostTag", |_args| {
        Ok(QuickJsHostValue::String("after restore".to_string()))
    })?;
    assert_eq!(restored.eval_string("hostTag()")?, "after restore");
    Ok(())
}
