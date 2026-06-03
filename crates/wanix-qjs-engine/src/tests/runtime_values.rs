use super::*;

#[test]
fn typed_eval_helpers_reject_wrong_result_types() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let number_err = vm
        .eval_number("({ valueOf() { throw new Error('nope'); } })")
        .expect_err("object result should not be read as a number");
    assert!(number_err.to_string().contains("not a number"));

    let string_err = vm
        .eval_string("({ toString() { throw new Error('nope'); } })")
        .expect_err("object result should not be read as a string");
    assert!(string_err.to_string().contains("not a string"));
    Ok(())
}

#[test]
fn runtime_recovers_after_eval_and_call_exceptions() -> Result<()> {
    let (engine, module) = quickjs_fixture()?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let eval_err = vm
        .eval_discard("throw new Error('eval boom')")
        .expect_err("thrown eval exception should surface to Rust");
    let eval_message = eval_err.to_string();
    assert!(eval_message.contains("QuickJS exception"));
    assert!(eval_message.contains("eval boom"));
    assert_eq!(vm.eval_number("21 * 2")?, 42.0);

    vm.eval_discard(
        r#"
        globalThis.throwFromCall = value => {
          throw new Error("call boom: " + value);
        };
        "#,
    )?;
    let call_err = vm
        .call_global_function_with_string("throwFromCall", "payload")
        .expect_err("thrown call exception should surface to Rust");
    let call_message = call_err.to_string();
    assert!(call_message.contains("QuickJS exception"));
    assert!(call_message.contains("call boom: payload"));
    assert_eq!(vm.eval_string("'still alive'")?, "still alive");
    Ok(())
}
