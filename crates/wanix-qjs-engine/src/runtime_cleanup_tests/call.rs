use crate::QuickJsValue;
use anyhow::Result;

use super::fault::CleanupFault::{FunctionValueFreeTrap, QjsCallTrap, QjsNewBigInt64Trap};
use super::fixture::{
    CleanupEvent::{CStringFree, GuestFree, ValueFree},
    CleanupValue::{BigInt, Call, Function, Global, StringValue, Undefined},
    cleanup_runtime, cleanup_runtime_with_fault, expect_cleanup_log,
};

#[test]
fn call_global_function_with_string_frees_guest_allocations_and_values() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    vm.call_global_function_with_string("fnName", "payload")?;

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            ValueFree(Global),
            GuestFree(4103),
            GuestFree(4111),
            ValueFree(Function),
            ValueFree(Undefined),
            ValueFree(StringValue),
            ValueFree(Call),
        ],
    )?;
    Ok(())
}

#[test]
fn call_global_function_frees_scalar_arguments_result_and_values() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    assert_eq!(
        vm.call_global_function("fnName", &[QuickJsValue::String("payload".into())])?,
        QuickJsValue::String("ok".into())
    );

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            ValueFree(Global),
            GuestFree(4103),
            GuestFree(4111),
            CStringFree(1216),
            ValueFree(Call),
            ValueFree(Function),
            ValueFree(Undefined),
            ValueFree(StringValue),
        ],
    )?;
    Ok(())
}

#[test]
fn call_global_function_frees_bigint_arguments_result_and_values() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    assert_eq!(
        vm.call_global_function("fnName", &[QuickJsValue::BigIntI64(42)])?,
        QuickJsValue::String("ok".into())
    );

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            ValueFree(Global),
            GuestFree(4103),
            CStringFree(1216),
            ValueFree(Call),
            ValueFree(Function),
            ValueFree(Undefined),
            ValueFree(BigInt),
        ],
    )?;
    Ok(())
}

#[test]
fn call_global_function_bigint_creation_trap_frees_function_and_this_values() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsNewBigInt64Trap)?;

    let err = vm
        .call_global_function("fnName", &[QuickJsValue::BigIntI64(42)])
        .expect_err("trapping qjs_new_big_int64 should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to create BigInt handle"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            ValueFree(Global),
            ValueFree(Function),
            ValueFree(Undefined),
        ],
    )?;
    Ok(())
}

#[test]
fn call_global_function_reports_first_cleanup_failure_and_continues_batch() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(FunctionValueFreeTrap)?;

    let err = vm
        .call_global_function_with_string("fnName", "payload")
        .expect_err("successful call should report cleanup failure");

    let message = format!("{err:#}");
    assert!(message.contains("failed to free QuickJS value"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            ValueFree(Global),
            GuestFree(4103),
            GuestFree(4111),
            ValueFree(Function),
            ValueFree(Undefined),
            ValueFree(StringValue),
            ValueFree(Call),
        ],
    )?;
    Ok(())
}

#[test]
fn call_global_function_trap_frees_argv_buffer_and_values() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsCallTrap)?;

    let err = vm
        .call_global_function_with_string("fnName", "payload")
        .expect_err("trapping qjs_call should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("failed to call QuickJS function"));
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            ValueFree(Global),
            GuestFree(4103),
            GuestFree(4111),
            ValueFree(Function),
            ValueFree(Undefined),
            ValueFree(StringValue),
        ],
    )?;
    Ok(())
}
