use crate::{QuickJsBinaryValue, QuickJsTypedArrayKind};
use anyhow::Result;

use super::fault::CleanupFault::{
    QjsGetArrayBufferReturnsNull, QjsGetArrayBufferTrap, QjsNewArrayBufferTrap, QjsNewDataViewTrap,
    QjsNewTypedArrayTrap,
};
use super::fixture::{
    CleanupEvent::{CStringFree, GuestFree, ValueFree},
    CleanupValue::{Eval, Exception, Global},
    cleanup_runtime, cleanup_runtime_with_fault, expect_cleanup_log,
};

#[test]
fn eval_binary_value_frees_guest_strings_len_slot_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    assert_eq!(
        vm.eval_binary_value("'ignored by synthetic runtime'")?,
        QuickJsBinaryValue::ArrayBuffer(b"<r".to_vec())
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
fn eval_binary_value_getter_trap_frees_len_slot_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsGetArrayBufferTrap)?;

    let err = vm
        .eval_binary_value("'ignored by synthetic runtime'")
        .expect_err("trapping qjs_get_array_buffer should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("qjs_get_array_buffer"));
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
fn eval_binary_value_null_data_pointer_takes_exception_and_frees_len_slot() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsGetArrayBufferReturnsNull)?;

    let err = vm
        .eval_binary_value("'ignored by synthetic runtime'")
        .expect_err("null qjs_get_array_buffer data pointer should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("qjs_get_array_buffer failed"));
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
fn set_global_binary_value_frees_input_name_global_and_value() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    vm.set_global_binary_value("bin", QuickJsBinaryValue::array_buffer(b"payload")?)?;

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4103),
            ValueFree(Global),
            ValueFree(Eval),
        ],
    )?;
    Ok(())
}

#[test]
fn set_global_binary_value_create_trap_frees_input_buffer() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsNewArrayBufferTrap)?;

    let err = vm
        .set_global_binary_value("bin", QuickJsBinaryValue::array_buffer(b"payload")?)
        .expect_err("trapping qjs_new_array_buffer should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("qjs_new_array_buffer"));
    expect_cleanup_log(&mut vm, &[GuestFree(4096)])?;
    Ok(())
}

#[test]
fn set_global_typed_array_value_create_trap_frees_input_buffer() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsNewTypedArrayTrap)?;

    let err = vm
        .set_global_binary_value(
            "bin",
            QuickJsBinaryValue::typed_array(QuickJsTypedArrayKind::Int16, b"data")?,
        )
        .expect_err("trapping qjs_new_typed_array should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("qjs_new_typed_array"));
    expect_cleanup_log(&mut vm, &[GuestFree(4096)])?;
    Ok(())
}

#[test]
fn set_global_data_view_value_create_trap_frees_input_buffer() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsNewDataViewTrap)?;

    let err = vm
        .set_global_binary_value("bin", QuickJsBinaryValue::data_view(b"data")?)
        .expect_err("trapping qjs_new_data_view should surface an error");

    let message = format!("{err:#}");
    assert!(message.contains("qjs_new_data_view"));
    expect_cleanup_log(&mut vm, &[GuestFree(4096)])?;
    Ok(())
}
