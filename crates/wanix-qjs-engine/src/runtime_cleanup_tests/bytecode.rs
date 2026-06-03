use anyhow::Result;

use super::{
    fault::CleanupFault::{
        QjsCompileReturnsNull, QjsCompileReturnsOversizedBuffer, QjsCompileTrap,
        QjsFreeBytecodeTrap,
    },
    fixture::{
        CleanupEvent::{BytecodeFree, CStringFree, GuestFree, ValueFree},
        CleanupValue::{Call, Exception},
        cleanup_runtime, cleanup_runtime_with_fault, expect_cleanup_log,
    },
};

#[test]
fn compile_bytecode_frees_source_filename_out_len_and_output_buffer() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    let bytecode = vm.compile_bytecode_with_options(
        "123456",
        "12345",
        crate::QuickJsBytecodeCompileOptions::new(),
    )?;
    assert_eq!(bytecode.bytes().len(), 2);

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4103),
            GuestFree(4109),
            BytecodeFree(4127),
        ],
    )?;
    Ok(())
}

#[test]
fn compile_bytecode_trap_frees_source_filename_and_out_len() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsCompileTrap)?;

    let err = vm
        .compile_bytecode_with_options(
            "123456",
            "12345",
            crate::QuickJsBytecodeCompileOptions::new(),
        )
        .expect_err("trapping qjs_compile should fail");
    assert!(format!("{err:#}").contains("failed to call qjs_compile"));

    expect_cleanup_log(
        &mut vm,
        &[GuestFree(4096), GuestFree(4103), GuestFree(4109)],
    )?;
    Ok(())
}

#[test]
fn compile_bytecode_null_frees_inputs_and_exception_string() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsCompileReturnsNull)?;

    let err = vm
        .compile_bytecode_with_options(
            "123456",
            "12345",
            crate::QuickJsBytecodeCompileOptions::new(),
        )
        .expect_err("null qjs_compile result should fail");
    assert!(format!("{err:#}").contains("QuickJS bytecode compilation failed"));

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4103),
            GuestFree(4109),
            CStringFree(1216),
            ValueFree(Exception),
        ],
    )?;
    Ok(())
}

#[test]
fn compile_bytecode_frees_output_buffer_when_guest_read_fails() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsCompileReturnsOversizedBuffer)?;

    let err = vm
        .compile_bytecode_with_options(
            "123456",
            "12345",
            crate::QuickJsBytecodeCompileOptions::new(),
        )
        .expect_err("out-of-bounds returned bytecode buffer should fail");
    assert!(err.to_string().contains("outside memory length"));

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4103),
            GuestFree(4109),
            BytecodeFree(4127),
        ],
    )?;
    Ok(())
}

#[test]
fn compile_bytecode_reports_bytecode_buffer_cleanup_failure() -> Result<()> {
    let mut vm = cleanup_runtime_with_fault(QjsFreeBytecodeTrap)?;

    let err = vm
        .compile_bytecode_with_options(
            "123456",
            "12345",
            crate::QuickJsBytecodeCompileOptions::new(),
        )
        .expect_err("bytecode buffer cleanup trap should fail compilation");
    assert!(format!("{err:#}").contains("failed to call qjs_free_bytecode"));

    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4103),
            GuestFree(4109),
            BytecodeFree(4127),
        ],
    )?;
    Ok(())
}

#[test]
fn eval_bytecode_discard_frees_input_buffer_and_result_value() -> Result<()> {
    let mut vm = cleanup_runtime()?;

    let bytecode = vm.compile_bytecode_with_options(
        "123456",
        "12345",
        crate::QuickJsBytecodeCompileOptions::new(),
    )?;
    expect_cleanup_log(
        &mut vm,
        &[
            GuestFree(4096),
            GuestFree(4103),
            GuestFree(4109),
            BytecodeFree(4127),
        ],
    )?;

    vm.eval_bytecode_discard(&bytecode)?;

    expect_cleanup_log(&mut vm, &[GuestFree(4113), ValueFree(Call)])?;
    Ok(())
}
