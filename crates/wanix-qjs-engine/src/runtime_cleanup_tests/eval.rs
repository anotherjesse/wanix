use crate::QuickJsValue;
use anyhow::Result;

use super::fault::CleanupFault::{
    CStringDataInvalidUtf8, CStringFreeTrap, EvalValueFreeTrap, QjsEvalReturnsNull, QjsEvalTrap,
    QjsGetBigInt64ReturnsFailure, QjsGetBigInt64Trap, QjsGetFloat64Trap, QjsGetStringReturnsNull,
    QjsGetStringTrap, QjsIsBigIntReturnsTrue, QjsIsNumberReturnsTrue, QjsIsNumberTrap,
    QjsIsStringReturnsFalse, QjsIsStringTrap,
};
use super::fixture::{
    CleanupEvent::{CStringFree, GuestFree, ValueFree},
    CleanupValue::{Eval, Exception},
    cleanup_runtime, cleanup_runtime_with_fault, cleanup_runtime_with_faults, expect_cleanup_log,
};

#[test]
fn eval_string_frees_guest_strings_c_string_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    assert_eq!(vm.eval_string("'ignored by synthetic runtime'")?, "ok");

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_value_frees_guest_strings_c_string_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    assert_eq!(
        vm.eval_value("'ignored by synthetic runtime'")?,
        QuickJsValue::String("ok".into())
    );

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_value_reads_bigint_and_frees_output_slot_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[QjsIsStringReturnsFalse, QjsIsBigIntReturnsTrue])?;

    assert_eq!(
        vm.eval_value("'ignored by synthetic runtime'")?,
        QuickJsValue::BigIntI64((1_i64 << 32) + 42)
    );

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            GuestFree(4134),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_value_bigint_read_trap_frees_output_slot_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[
        QjsIsStringReturnsFalse,
        QjsIsBigIntReturnsTrue,
        QjsGetBigInt64Trap,
    ])?;

    let err = vm
        .eval_value("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_get_big_int64 should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to read BigInt result"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            GuestFree(4134),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_value_bigint_failure_frees_exception_output_slot_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[
        QjsIsStringReturnsFalse,
        QjsIsBigIntReturnsTrue,
        QjsGetBigInt64ReturnsFailure,
    ])?;

    let err = vm
        .eval_value("'ignored by synthetic runtime'")
        .expect_err("failing qjs_get_big_int64 should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("qjs_get_big_int64 failed: ok"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Exception),
            GuestFree(4134),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_number_primary_error_wins_over_value_cleanup_failure() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(EvalValueFreeTrap)?;

    let err = vm
        .eval_number("'ignored by synthetic runtime'")
        .expect_err("wrong result type should surface before cleanup failure");

    let message = format!("{err:#}");
    assert!(message.contains("QuickJS evaluation result is not a number"));
    assert!(!message.contains("failed to free QuickJS value"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn eval_string_reports_c_string_cleanup_failure_and_frees_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(CStringFreeTrap)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("successful string eval should report cleanup failure");

    let message = format!("{err:#}");
    assert!(message.contains("failed to free QuickJS C string"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_number_inspection_trap_frees_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsIsNumberTrap)?;

    let err = vm
        .eval_number("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_is_number should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to inspect number result"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn eval_number_read_trap_frees_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[QjsIsNumberReturnsTrue, QjsGetFloat64Trap])?;

    let err = vm
        .eval_number("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_get_float64 should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to read number result"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn eval_string_inspection_trap_frees_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsIsStringTrap)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_is_string should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to inspect string result"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn eval_string_conversion_trap_frees_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsGetStringTrap)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_get_string should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to convert result to string"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn eval_string_read_c_string_failure_frees_c_string_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(CStringDataInvalidUtf8)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("invalid C string bytes should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("guest string was not valid UTF-8"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_string_read_error_wins_over_c_string_cleanup_failure() -> Result<()> {
    let mut vm = cleanup_runtime_with_faults(&[CStringDataInvalidUtf8, CStringFreeTrap])?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("invalid C string bytes should surface before cleanup failure");

    let message = format!("{err:#}");
    assert!(message.contains("guest string was not valid UTF-8"));
    assert!(!message.contains("failed to free QuickJS C string"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4127),
            CStringFree(1216),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_string_null_c_string_frees_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsGetStringReturnsNull)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("null qjs_get_string result should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("QuickJS failed to convert string result to a C string"));
    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4127), ValueFree(Eval)],
    )?;
    Ok(())
}

#[test]
fn eval_trap_frees_guest_strings() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsEvalTrap)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_eval should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to call qjs_eval"));
    expect_cleanup_log(&mut vm, &[GuestFree(4096), GuestFree(4127)])?;
    Ok(())
}

#[test]
fn eval_null_result_frees_guest_strings() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsEvalReturnsNull)?;

    let err = vm
        .eval_string("'ignored by synthetic runtime'")
        .expect_err("null qjs_eval result should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("qjs_eval returned a null JSValue pointer"));
    expect_cleanup_log(&mut vm, &[GuestFree(4096), GuestFree(4127)])?;
    Ok(())
}
