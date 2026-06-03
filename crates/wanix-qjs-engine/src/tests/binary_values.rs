use super::*;

#[test]
fn eval_binary_value_returns_array_buffer_and_uint8_array_copies() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    assert_eq!(
        vm.eval_binary_value(
            r#"
            const buffer = new ArrayBuffer(4);
            new Uint8Array(buffer).set([0xde, 0xad, 0xbe, 0xef]);
            buffer
            "#,
        )?,
        QuickJsBinaryValue::ArrayBuffer(vec![0xde, 0xad, 0xbe, 0xef])
    );

    assert_eq!(
        vm.eval_binary_value("new Uint8Array([1, 2, 3])")?,
        QuickJsBinaryValue::Uint8Array(vec![1, 2, 3])
    );

    assert_eq!(
        vm.eval_binary_value("new Uint8Array([1, 2, 3, 4, 5]).subarray(1, 4)")?,
        QuickJsBinaryValue::Uint8Array(vec![2, 3, 4])
    );

    assert_eq!(
        vm.eval_binary_value(
            r#"
            (() => {
              const buffer = new ArrayBuffer(8);
              new Uint8Array(buffer).set([1, 2, 3, 4, 5, 6, 7, 8]);
              return new Int16Array(buffer, 2, 2);
            })()
            "#,
        )?,
        QuickJsBinaryValue::TypedArray {
            kind: QuickJsTypedArrayKind::Int16,
            bytes: vec![3, 4, 5, 6],
        }
    );

    assert_eq!(
        vm.eval_binary_value("new Uint8ClampedArray([1, 2, 3]).subarray(1)")?,
        QuickJsBinaryValue::TypedArray {
            kind: QuickJsTypedArrayKind::Uint8Clamped,
            bytes: vec![2, 3],
        }
    );

    assert_eq!(
        vm.eval_binary_value(
            r#"
            (() => {
              const buffer = new ArrayBuffer(6);
              new Uint8Array(buffer).set([9, 8, 7, 6, 5, 4]);
              return new DataView(buffer, 1, 4);
            })()
            "#,
        )?,
        QuickJsBinaryValue::DataView(vec![8, 7, 6, 5])
    );
    Ok(())
}

#[test]
fn set_and_get_global_binary_values_round_trip_copied_bytes() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_global_binary_value(
        "buffer",
        QuickJsBinaryValue::array_buffer(&[10, 20, 30, 40])?,
    )?;
    assert_eq!(
        vm.eval_string("buffer instanceof ArrayBuffer ? 'yes' : 'no'")?,
        "yes"
    );
    vm.eval_discard("new Uint8Array(buffer)[2] = 99")?;
    assert_eq!(
        vm.get_global_binary_value("buffer")?,
        QuickJsBinaryValue::ArrayBuffer(vec![10, 20, 99, 40])
    );

    vm.set_global_binary_value("bytes", QuickJsBinaryValue::uint8_array(&[5, 6, 7])?)?;
    assert_eq!(
        vm.eval_string("bytes instanceof Uint8Array ? 'yes' : 'no'")?,
        "yes"
    );
    vm.eval_discard("bytes.reverse()")?;
    assert_eq!(
        vm.get_global_binary_value("bytes")?,
        QuickJsBinaryValue::Uint8Array(vec![7, 6, 5])
    );

    vm.set_global_binary_value(
        "ints",
        QuickJsBinaryValue::typed_array(QuickJsTypedArrayKind::Int16, &[1, 0, 255, 127])?,
    )?;
    assert_eq!(
        vm.eval_string(
            r#"
            `${ints.constructor.name}:${ints.length}:${
              Array.from(new Uint8Array(ints.buffer, ints.byteOffset, ints.byteLength)).join(",")
            }`
            "#,
        )?,
        "Int16Array:2:1,0,255,127"
    );
    assert_eq!(
        vm.get_global_binary_value("ints")?,
        QuickJsBinaryValue::TypedArray {
            kind: QuickJsTypedArrayKind::Int16,
            bytes: vec![1, 0, 255, 127],
        }
    );

    vm.set_global_binary_value("view", QuickJsBinaryValue::data_view(&[3, 1, 4, 1])?)?;
    assert_eq!(
        vm.eval_string(
            r#"
            `${view instanceof DataView}:${view.byteLength}:${
              Array.from(new Uint8Array(view.buffer, view.byteOffset, view.byteLength)).join(",")
            }`
            "#,
        )?,
        "true:4:3,1,4,1"
    );
    vm.eval_discard("view.setUint8(2, 9)")?;
    assert_eq!(
        vm.get_global_binary_value("view")?,
        QuickJsBinaryValue::DataView(vec![3, 1, 9, 1])
    );
    Ok(())
}

#[test]
fn binary_values_handle_empty_and_large_uint8_arrays() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_global_binary_value("empty", QuickJsBinaryValue::uint8_array(&[])?)?;
    assert_eq!(
        vm.get_global_binary_value("empty")?,
        QuickJsBinaryValue::Uint8Array(Vec::new())
    );

    let large = (0..65_000)
        .map(|index| u8::try_from(index % 251).expect("test byte should fit"))
        .collect::<Vec<_>>();
    vm.set_global_binary_value("large", QuickJsBinaryValue::uint8_array(&large)?)?;
    assert_eq!(vm.eval_number("large.length")?, 65_000.0);
    assert_eq!(
        vm.get_global_binary_value("large")?.bytes(),
        large.as_slice()
    );
    Ok(())
}

#[test]
fn call_global_function_binary_uses_scalar_args_and_copies_binary_result() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard(
        r#"
        globalThis.makeBytes = (base, count) => {
          const bytes = new Uint8Array(count);
          for (let index = 0; index < count; index++) bytes[index] = base + index;
          return bytes.subarray(1, count - 1);
        };
        "#,
    )?;

    assert_eq!(
        vm.call_global_function_binary(
            "makeBytes",
            &[QuickJsValue::Number(40.0), QuickJsValue::Number(4.0)],
        )?,
        QuickJsBinaryValue::Uint8Array(vec![41, 42])
    );
    Ok(())
}

#[test]
fn call_global_function_with_values_uses_scalar_and_binary_args() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard(
        r#"
        globalThis.describeBytes = (label, bytes, buffer) => {
          const view = new Uint8Array(buffer);
          return `${label}:${Array.from(bytes).join(",")}:${view[1]}`;
        };
        globalThis.mixBytes = (bytes, extra) => {
          const output = new Uint8Array(bytes.length + 2);
          output.set(bytes, 0);
          output.set([extra, extra + 1], bytes.length);
          return output.subarray(1);
        };
        globalThis.describeTyped = (view) => {
          const raw = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
          return `${view.constructor.name}:${Array.from(raw).join("-")}`;
        };
        globalThis.rewrapTyped = (view) => {
          return new Uint16Array(view.buffer, view.byteOffset, view.byteLength / 2);
        };
        globalThis.describeDataView = (view) => {
          return `${view instanceof DataView}:${
            Array.from(new Uint8Array(view.buffer, view.byteOffset, view.byteLength)).join("-")
          }`;
        };
        globalThis.rewrapDataView = (view) => {
          return new DataView(view.buffer, 1, 2);
        };
        globalThis.bumpBigIntAndDescribeBytes = (value, bytes) => {
          return `${value + 2n}:${bytes[0]}`;
        };
        globalThis.returnMixedBigInt = (bytes) => {
          return BigInt(bytes[0]) + 40n;
        };
        "#,
    )?;

    assert_eq!(
        vm.call_global_function_with_values(
            "describeBytes",
            &[
                QuickJsValue::String("payload".into()).into(),
                QuickJsBinaryValue::uint8_array(&[1, 2, 3])?.into(),
                QuickJsBinaryValue::array_buffer(&[9, 8])?.into(),
            ],
        )?,
        QuickJsCopiedValue::Scalar(QuickJsValue::String("payload:1,2,3:8".into()))
    );

    assert_eq!(
        vm.call_global_function_with_values(
            "mixBytes",
            &[
                QuickJsBinaryValue::uint8_array(&[10, 20, 30])?.into(),
                QuickJsValue::Number(40.0).into(),
            ],
        )?,
        QuickJsCopiedValue::Binary(QuickJsBinaryValue::Uint8Array(vec![20, 30, 40, 41]))
    );

    assert_eq!(
        vm.call_global_function_with_values(
            "describeTyped",
            &[
                QuickJsBinaryValue::typed_array(QuickJsTypedArrayKind::Int16, &[1, 2, 3, 4])?
                    .into()
            ],
        )?,
        QuickJsCopiedValue::Scalar(QuickJsValue::String("Int16Array:1-2-3-4".into()))
    );

    assert_eq!(
        vm.call_global_function_with_values(
            "rewrapTyped",
            &[
                QuickJsBinaryValue::typed_array(QuickJsTypedArrayKind::Int16, &[9, 8, 7, 6])?
                    .into()
            ],
        )?,
        QuickJsCopiedValue::Binary(QuickJsBinaryValue::TypedArray {
            kind: QuickJsTypedArrayKind::Uint16,
            bytes: vec![9, 8, 7, 6],
        })
    );

    assert_eq!(
        vm.call_global_function_with_values(
            "describeDataView",
            &[QuickJsBinaryValue::data_view(&[4, 5, 6])?.into()],
        )?,
        QuickJsCopiedValue::Scalar(QuickJsValue::String("true:4-5-6".into()))
    );

    assert_eq!(
        vm.call_global_function_with_values(
            "rewrapDataView",
            &[QuickJsBinaryValue::data_view(&[8, 7, 6, 5])?.into()],
        )?,
        QuickJsCopiedValue::Binary(QuickJsBinaryValue::DataView(vec![7, 6]))
    );

    assert_eq!(
        vm.call_global_function_with_values(
            "bumpBigIntAndDescribeBytes",
            &[
                QuickJsValue::BigIntI64(40).into(),
                QuickJsBinaryValue::uint8_array(&[9])?.into(),
            ],
        )?,
        QuickJsCopiedValue::Scalar(QuickJsValue::String("42:9".into()))
    );

    assert_eq!(
        vm.call_global_function_with_values(
            "returnMixedBigInt",
            &[QuickJsBinaryValue::uint8_array(&[2])?.into()],
        )?,
        QuickJsCopiedValue::Scalar(QuickJsValue::BigIntI64(42))
    );
    Ok(())
}

#[test]
fn restored_runtime_preserves_binary_globals() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.set_global_binary_value("buffer", QuickJsBinaryValue::array_buffer(&[1, 2, 3])?)?;
    vm.set_global_binary_value("bytes", QuickJsBinaryValue::uint8_array(&[9, 8, 7])?)?;
    vm.set_global_binary_value(
        "words",
        QuickJsBinaryValue::typed_array(QuickJsTypedArrayKind::Uint16, &[10, 0, 20, 0])?,
    )?;
    vm.set_global_binary_value("view", QuickJsBinaryValue::data_view(&[5, 6, 7, 8])?)?;
    vm.eval_discard(
        r#"
        new Uint8Array(buffer)[1] = 22;
        bytes[2] = 66;
        new Uint8Array(words.buffer, words.byteOffset, words.byteLength)[2] = 33;
        view.setUint8(3, 44);
        "#,
    )?;
    let snapshot = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot)?;
    assert_eq!(
        restored.get_global_binary_value("buffer")?,
        QuickJsBinaryValue::ArrayBuffer(vec![1, 22, 3])
    );
    assert_eq!(
        restored.get_global_binary_value("bytes")?,
        QuickJsBinaryValue::Uint8Array(vec![9, 8, 66])
    );
    assert_eq!(
        restored.get_global_binary_value("words")?,
        QuickJsBinaryValue::TypedArray {
            kind: QuickJsTypedArrayKind::Uint16,
            bytes: vec![10, 0, 33, 0],
        }
    );
    assert_eq!(
        restored.get_global_binary_value("view")?,
        QuickJsBinaryValue::DataView(vec![5, 6, 7, 44])
    );
    Ok(())
}

#[test]
fn restored_runtime_calls_global_function_with_copied_values() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard(
        r#"
        globalThis.sumBytes = (bytes) => {
          let total = 0;
          for (const byte of bytes) total += byte;
          return total;
        };
        "#,
    )?;
    let snapshot = vm.snapshot()?.try_to_bytes()?;
    drop(vm);

    let mut restored = module.restore_runtime_from_bytes(&snapshot)?;
    assert_eq!(
        restored.call_global_function_with_values(
            "sumBytes",
            &[QuickJsBinaryValue::uint8_array(&[5, 6, 7])?.into()],
        )?,
        QuickJsCopiedValue::Scalar(QuickJsValue::Number(18.0))
    );
    Ok(())
}

#[test]
fn binary_value_errors_recover_runtime_and_reject_nul_global_names() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.eval_discard("globalThis.returnObject = () => ({ not: 'copied' })")?;
    let err = vm
        .call_global_function_with_values("returnObject", &[])
        .expect_err("plain objects should not be copied as scalar or binary values");
    assert!(
        err.to_string()
            .contains("not a supported copied scalar or binary value")
    );
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);

    let err = vm
        .eval_binary_value("({ not: 'bytes' })")
        .expect_err("plain objects should not be copied as binary values");
    assert!(
        err.to_string()
            .contains("not a supported copied binary value")
    );
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);

    let err = QuickJsBinaryValue::typed_array(QuickJsTypedArrayKind::Int16, &[1])
        .expect_err("typed array bytes must align to element width");
    assert!(err.to_string().contains("not a multiple"));

    let err = vm
        .set_global_binary_value(
            "badTyped",
            QuickJsBinaryValue::TypedArray {
                kind: QuickJsTypedArrayKind::Int16,
                bytes: vec![1],
            },
        )
        .expect_err("direct enum construction should still validate typed array width");
    assert!(err.to_string().contains("not a multiple"));
    assert_eq!(vm.eval_number("6 * 7")?, 42.0);

    let value = QuickJsBinaryValue::uint8_array(&[1, 2, 3])?;
    let err = vm
        .set_global_binary_value("bad\0name", value)
        .expect_err("embedded NULs should not silently truncate global writes");
    assert!(err.to_string().contains("must not contain NUL"));

    let err = vm
        .get_global_binary_value("bad\0name")
        .expect_err("embedded NULs should not silently truncate global reads");
    assert!(err.to_string().contains("must not contain NUL"));

    let err = vm
        .call_global_function_binary("bad\0name", &[])
        .expect_err("embedded NULs should not silently truncate global calls");
    assert!(err.to_string().contains("must not contain NUL"));

    let err = vm
        .call_global_function_with_values("bad\0name", &[])
        .expect_err("embedded NULs should not silently truncate mixed global calls");
    assert!(err.to_string().contains("must not contain NUL"));
    Ok(())
}
