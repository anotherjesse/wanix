use anyhow::Result;

use super::fault::CleanupFault::{
    CStringDataInvalidUtf8, QjsCallReturnsException, QjsEvalReturnsException, QjsGetExceptionTrap,
    QjsGetStringReturnsNull, QjsGetStringTrap, QjsIsExceptionTrap,
};
use super::fixture::{
    CleanupEvent::{CStringFree, GuestFree, ValueFree},
    CleanupValue::{Call, Eval, Exception, Function, Global, StringValue, Undefined},
    cleanup_runtime_with_fault, cleanup_runtime_with_faults, expect_cleanup_log,
};

#[test]
fn exception_inspection_trap_frees_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsIsExceptionTrap)?;

    let err = vm
        .eval_discard("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_is_exception should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to inspect QuickJS value"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn eval_exception_frees_guest_strings_exception_resources_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsEvalReturnsException)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("exceptional qjs_eval result should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("QuickJS exception"));
    assert!(message.contains("ok"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Exception),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn exception_take_trap_frees_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[QjsEvalReturnsException, QjsGetExceptionTrap])?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_get_exception should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to take QuickJS exception"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn exception_stringification_trap_frees_exception_and_result_values() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[QjsEvalReturnsException, QjsGetStringTrap])?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("trapping exception stringification should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to stringify QuickJS exception"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            ValueFree(Exception),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn exception_read_c_string_failure_frees_c_string_exception_and_result_values() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[QjsEvalReturnsException, CStringDataInvalidUtf8])?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("invalid exception C string bytes should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("guest string was not valid UTF-8"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Exception),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn exception_null_c_string_frees_exception_and_result_values() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[QjsEvalReturnsException, QjsGetStringReturnsNull])?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("null exception C string should surface fallback exception text");

    let message = format!("{err:#}");
    assert!(message.contains("QuickJS exception"));
    assert!(message.contains("<failed to stringify QuickJS exception>"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            ValueFree(Exception),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn call_global_function_exception_frees_argv_exception_resources_and_values() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsCallReturnsException)?;

    let err = vm
        .call_global_function_with_string("fnName", "payload")
        .expect_err("exceptional qjs_call result should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("QuickJS exception"));
    assert!(message.contains("ok"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            ValueFree(Global),
            GuestFree(4103),
            GuestFree(4111),
            CStringFree(1216),
            ValueFree(Exception),
            ValueFree(Call),
            ValueFree(Function),
            ValueFree(Undefined),
            ValueFree(StringValue),
        ],
    )?;
    Ok(())
}
