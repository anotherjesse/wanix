use crate::{
    QuickJsBinaryValue, QuickJsBytecode, QuickJsCallbackValue, QuickJsCopiedValue,
    QuickJsHostValue, QuickJsIntrinsics, QuickJsRuntime, QuickJsValue,
};
use anyhow::Result;
use fixture::{
    MINIMAL_ABI_WAT, assert_module_error, assert_module_error_contains,
    assert_module_error_with_engine, engine_with_config, module_from_wat,
    module_from_wat_with_engine, replace_once, with_extra_export,
};

mod fixture;

#[test]
fn module_preflight_accepts_minimal_quickjs_abi() -> Result<()> {
    module_from_wat(MINIMAL_ABI_WAT)?;

    Ok(())
}

#[test]
fn module_preflight_allows_extra_exports() -> Result<()> {
    let wat = with_extra_export();

    module_from_wat(&wat)?;

    Ok(())
}

#[test]
fn host_callback_api_reports_missing_optional_exports_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, MINIMAL_ABI_WAT)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .define_global_host_function("hostFn", |_args| Ok(QuickJsHostValue::Undefined))
        .expect_err("minimal ABI should not support host callbacks");
    assert!(err.to_string().contains("qjs_is_undefined"));
    assert!(err.to_string().contains("host callbacks are not supported"));
    Ok(())
}

#[test]
fn host_callback_capability_does_not_require_bool_or_null_helpers() -> Result<()> {
    let prefix = MINIMAL_ABI_WAT
        .trim_end()
        .strip_suffix(')')
        .expect("fixture module should end with a closing paren");
    let wat = format!(
        r#"{prefix}
  (func (export "qjs_is_undefined") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_new_number") (param $value f64) (result i32)
    i32.const 44)
  (func (export "qjs_throw") (param $value i32) (result i32)
    i32.const 55)
  (func (export "qjs_new_host_function") (param $name i32) (param $name_len i32) (param $reserved i32) (result i32)
    i32.const 88)
  (func (export "qjs_set_prop_string") (param $global i32) (param $name i32) (param $value i32) (result i32)
    i32.const 0)
)
"#
    );
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, &wat)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    vm.define_global_host_function("hostFn", |_args| Ok(QuickJsHostValue::Undefined))?;

    let err = vm
        .register_host_callback_with_binary_values("hostBinary", |_args| {
            Ok(QuickJsCallbackValue::Scalar(QuickJsValue::Undefined))
        })
        .expect_err("binary-capable callbacks should require binary helpers");
    assert!(err.to_string().contains("qjs_new_array_buffer"));
    assert!(err.to_string().contains("binary values are not supported"));
    Ok(())
}

#[test]
fn scalar_value_api_reports_missing_optional_exports_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    i32.const 4)"#,
        r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    unreachable)"#,
    );
    let wat = replace_once(
        &wat,
        r#"  (func (export "qjs_get_prop_string") (param $global i32) (param $name i32) (result i32)
    i32.const 20)"#,
        r#"  (func (export "qjs_get_prop_string") (param $global i32) (param $name i32) (result i32)
    unreachable)"#,
    );
    let module = module_from_wat_with_engine(&engine, &wat)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .eval_value("undefined")
        .expect_err("unsupported scalar values should fail before evaluating JS");
    assert!(err.to_string().contains("qjs_is_undefined"));
    assert!(err.to_string().contains("scalar values are not supported"));

    let err = vm
        .get_global_value("value")
        .expect_err("unsupported scalar values should fail before reading globals");
    assert!(err.to_string().contains("qjs_is_undefined"));
    assert!(err.to_string().contains("scalar values are not supported"));

    let err = vm
        .set_global_value("value", QuickJsValue::Null)
        .expect_err("minimal ABI should not support copied scalar values");
    assert!(err.to_string().contains("qjs_is_undefined"));
    assert!(err.to_string().contains("scalar values are not supported"));

    let err = vm
        .call_global_function("fn", &[])
        .expect_err("minimal ABI should not support copied scalar values");
    assert!(err.to_string().contains("qjs_is_undefined"));
    assert!(err.to_string().contains("scalar values are not supported"));
    Ok(())
}

#[test]
fn bigint_scalar_value_reports_missing_export_on_older_scalar_abi() -> Result<()> {
    let prefix = MINIMAL_ABI_WAT
        .trim_end()
        .strip_suffix(')')
        .expect("fixture module should end with a closing paren");
    let wat = format!(
        r#"{prefix}
  (func (export "qjs_get_null") (result i32)
    i32.const 70)
  (func (export "qjs_get_true") (result i32)
    i32.const 80)
  (func (export "qjs_get_false") (result i32)
    i32.const 90)
  (func (export "qjs_is_undefined") (param $value i32) (result i32)
    local.get $value
    i32.const 4
    i32.eq)
  (func (export "qjs_is_null") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_bool") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_bool") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_new_number") (param $value f64) (result i32)
    i32.const 44)
  (func (export "qjs_set_prop_string") (param $global i32) (param $name i32) (param $value i32) (result i32)
    i32.const 0)
)
"#
    );
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, &wat)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    assert_eq!(vm.eval_value("undefined")?, QuickJsValue::Undefined);
    vm.set_global_value("number", QuickJsValue::Number(1.0))?;

    let err = vm
        .set_global_value("big", QuickJsValue::BigIntI64(42))
        .expect_err("older scalar helper ABI should not create BigInt values");
    assert!(err.to_string().contains("qjs_new_big_int64"));
    assert!(err.to_string().contains("scalar values are not supported"));
    Ok(())
}

#[test]
fn binary_value_api_reports_missing_optional_exports_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, MINIMAL_ABI_WAT)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .eval_binary_value("new Uint8Array()")
        .expect_err("unsupported binary values should fail before evaluating JS");
    assert!(err.to_string().contains("qjs_new_array_buffer"));
    assert!(err.to_string().contains("binary values are not supported"));

    let err = vm
        .get_global_binary_value("value")
        .expect_err("unsupported binary values should fail before reading globals");
    assert!(err.to_string().contains("qjs_new_array_buffer"));
    assert!(err.to_string().contains("binary values are not supported"));

    let err = vm
        .set_global_binary_value("value", QuickJsBinaryValue::uint8_array(&[])?)
        .expect_err("minimal ABI should not support copied binary values");
    assert!(err.to_string().contains("qjs_new_array_buffer"));
    assert!(err.to_string().contains("binary values are not supported"));

    let err = vm
        .call_global_function_binary("value", &[])
        .expect_err("unsupported binary values should fail before reading functions");
    assert!(err.to_string().contains("qjs_new_array_buffer"));
    assert!(err.to_string().contains("binary values are not supported"));

    let err = vm
        .call_global_function_with_values(
            "value",
            &[QuickJsCopiedValue::Scalar(QuickJsValue::Undefined)],
        )
        .expect_err("unsupported mixed copied values should fail before reading functions");
    assert!(err.to_string().contains("qjs_new_array_buffer"));
    assert!(err.to_string().contains("binary values are not supported"));
    Ok(())
}

#[test]
fn data_view_value_reports_missing_export_on_older_binary_abi() -> Result<()> {
    let prefix = MINIMAL_ABI_WAT
        .trim_end()
        .strip_suffix(')')
        .expect("fixture module should end with a closing paren");
    let wat = format!(
        r#"{prefix}
  (func (export "qjs_new_array_buffer") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)
  (func (export "qjs_new_uint8_array") (param $ptr i32) (param $len i32) (result i32)
    i32.const 4)
  (func (export "qjs_is_array_buffer") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_is_uint8_array") (param $value i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_array_buffer") (param $value i32) (param $len_out i32) (result i32)
    i32.const 0)
  (func (export "qjs_get_uint8_array") (param $value i32) (param $len_out i32) (result i32)
    i32.const 0)
)
"#
    );
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, &wat)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .set_global_binary_value("value", QuickJsBinaryValue::data_view(&[1, 2, 3])?)
        .expect_err("older binary helper ABI should not create DataView values");
    assert!(err.to_string().contains("qjs_new_data_view"));
    assert!(err.to_string().contains("binary values are not supported"));
    Ok(())
}

#[test]
fn module_loader_api_reports_missing_optional_export_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, MINIMAL_ABI_WAT)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .set_module_loader(|_name| Ok(String::new()))
        .expect_err("minimal ABI should not support module loaders");
    assert!(err.to_string().contains("qjs_set_module_loader"));
    assert!(err.to_string().contains("module loaders are not supported"));
    Ok(())
}

#[test]
fn runtime_limit_apis_report_missing_optional_exports_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, MINIMAL_ABI_WAT)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .set_memory_limit(1)
        .expect_err("minimal ABI should not support memory limits");
    assert!(err.to_string().contains("qjs_set_memory_limit"));
    assert!(err.to_string().contains("runtime limits are not supported"));

    let err = vm
        .set_max_stack_size(1)
        .expect_err("minimal ABI should not support stack limits");
    assert!(err.to_string().contains("qjs_set_max_stack_size"));
    assert!(err.to_string().contains("runtime limits are not supported"));

    let err = vm
        .run_gc()
        .expect_err("minimal ABI should not support explicit GC");
    assert!(err.to_string().contains("qjs_run_gc"));
    assert!(err.to_string().contains("runtime limits are not supported"));

    let err = vm
        .set_gc_threshold(1)
        .expect_err("minimal ABI should not support GC thresholds");
    assert!(err.to_string().contains("qjs_set_gc_threshold"));
    assert!(err.to_string().contains("runtime limits are not supported"));

    let err = vm
        .gc_threshold()
        .expect_err("minimal ABI should not support reading GC thresholds");
    assert!(err.to_string().contains("qjs_get_gc_threshold"));
    assert!(err.to_string().contains("runtime limits are not supported"));

    let err = vm
        .memory_usage()
        .expect_err("minimal ABI should not support memory usage diagnostics");
    assert!(err.to_string().contains("qjs_compute_memory_usage"));
    assert!(err.to_string().contains("runtime limits are not supported"));

    let err = vm
        .set_interrupt_handler(|| false)
        .expect_err("minimal ABI should not support interrupt handlers");
    assert!(err.to_string().contains("qjs_set_interrupt_handler"));
    assert!(err.to_string().contains("runtime limits are not supported"));
    Ok(())
}

#[test]
fn runtime_limit_optional_exports_reject_wrong_signatures() -> Result<()> {
    let prefix = MINIMAL_ABI_WAT
        .trim_end()
        .strip_suffix(')')
        .expect("fixture module should end with a closing paren");
    let wat = format!(
        r#"{prefix}
  (func (export "qjs_compute_memory_usage"))
)
"#
    );
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, &wat)?;

    let err = match QuickJsRuntime::create(&engine, &module) {
        Ok(_) => {
            anyhow::bail!("mistyped optional memory usage export should fail runtime creation")
        }
        Err(err) => err,
    };
    assert!(format!("{err:#}").contains("qjs_compute_memory_usage"));
    assert!(format!("{err:#}").contains("missing or mistyped"));
    Ok(())
}

#[test]
fn promise_rejection_api_reports_missing_optional_export_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, MINIMAL_ABI_WAT)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .set_promise_rejection_handler(|_event| {})
        .expect_err("minimal ABI should not support promise rejection tracking");
    assert!(
        err.to_string()
            .contains("qjs_set_promise_rejection_handler")
    );
    assert!(
        err.to_string()
            .contains("promise rejection tracking is not supported")
    );
    Ok(())
}

#[test]
fn bytecode_api_reports_missing_optional_exports_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, MINIMAL_ABI_WAT)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .compile_bytecode("1")
        .expect_err("minimal ABI should not support bytecode compilation");
    assert!(err.to_string().contains("qjs_compile"));
    assert!(err.to_string().contains("bytecode is not supported"));

    let bytecode = QuickJsBytecode::from_trusted_parts(module.wasm_sha256(), &[0xff])?;
    let err = vm
        .eval_bytecode_discard(&bytecode)
        .expect_err("minimal ABI should not support bytecode evaluation");
    assert!(err.to_string().contains("qjs_eval_bytecode"));
    assert!(err.to_string().contains("bytecode is not supported"));

    let err = vm
        .eval_bytecode_value(&bytecode)
        .expect_err("minimal ABI should report bytecode support before scalar helpers");
    assert!(err.to_string().contains("qjs_eval_bytecode"));
    assert!(err.to_string().contains("bytecode is not supported"));
    Ok(())
}

#[test]
fn bytecode_compile_requires_matching_free_export() -> Result<()> {
    let prefix = MINIMAL_ABI_WAT
        .trim_end()
        .strip_suffix(')')
        .expect("fixture module should end with a closing paren");
    let wat = format!(
        r#"{prefix}
  (func (export "qjs_compile")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $eval_flags i32)
    (param $write_flags i32)
    (param $out_len i32)
    (result i32)
    unreachable)
)
"#
    );
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, &wat)?;
    let mut vm = QuickJsRuntime::create(&engine, &module)?;

    let err = vm
        .compile_bytecode("1")
        .expect_err("bytecode compile should require a matching free export");
    assert!(err.to_string().contains("qjs_free_bytecode"));
    assert!(err.to_string().contains("bytecode is not supported"));
    Ok(())
}

#[test]
fn create_runtime_with_intrinsics_reports_missing_optional_export_on_minimal_abi() -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, MINIMAL_ABI_WAT)?;

    QuickJsRuntime::create(&engine, &module)?;

    let err = match module.create_runtime_with_intrinsics(QuickJsIntrinsics::EVAL) {
        Ok(_) => anyhow::bail!("minimal ABI should not support configurable intrinsics"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("qjs_init2"));
    assert!(err.to_string().contains("configurable intrinsics"));
    Ok(())
}

#[test]
fn create_runtime_with_intrinsics_rejects_mistyped_optional_export() -> Result<()> {
    let prefix = MINIMAL_ABI_WAT
        .trim_end()
        .strip_suffix(')')
        .expect("fixture module should end with a closing paren");
    let wat = format!(
        r#"{prefix}
  (func (export "qjs_init2") (result i32)
    i32.const 0)
)
"#
    );
    let engine = wasmtime::Engine::default();
    let module = module_from_wat_with_engine(&engine, &wat)?;

    let err = match module.create_runtime_with_intrinsics(QuickJsIntrinsics::EVAL) {
        Ok(_) => anyhow::bail!("mistyped qjs_init2 should fail runtime creation"),
        Err(err) => err,
    };
    let message = format!("{err:#}");
    assert!(message.contains("qjs_init2"));
    assert!(message.contains("missing or mistyped"));
    Ok(())
}

#[test]
fn module_preflight_rejects_missing_memory() {
    let wat = replace_once(MINIMAL_ABI_WAT, "  (memory (export \"memory\") 1)\n", "");

    assert_module_error(&wat, "does not export memory");
}

#[test]
fn module_preflight_rejects_mistyped_memory_export() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        "(memory (export \"memory\") 1)",
        "(func (export \"memory\"))",
    );

    assert_module_error(&wat, "export memory has type function, expected memory");
}

#[test]
fn module_preflight_rejects_64_bit_memory() -> Result<()> {
    let engine = engine_with_config(|config| {
        config.wasm_memory64(true);
    })?;
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        "(memory (export \"memory\") 1)",
        "(memory (export \"memory\") i64 1)",
    );

    assert_module_error_with_engine(&engine, &wat, "memory export must use 32-bit indexes");
    Ok(())
}

#[test]
fn module_preflight_rejects_non_64k_page_memory() -> Result<()> {
    let engine = engine_with_config(|config| {
        config.wasm_custom_page_sizes(true);
    })?;
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        "(memory (export \"memory\") 1)",
        "(memory (export \"memory\") 1 (pagesize 1))",
    );

    assert_module_error_with_engine(&engine, &wat, "memory export must use 64 KiB pages");
    Ok(())
}

#[test]
fn module_preflight_rejects_missing_stack_pointer() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        "  (global $__stack_pointer (export \"__stack_pointer\") (mut i32) (i32.const 65536))\n",
        "",
    );

    assert_module_error(&wat, "does not export __stack_pointer");
}

#[test]
fn module_preflight_rejects_immutable_stack_pointer() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        "(global $__stack_pointer (export \"__stack_pointer\") (mut i32) (i32.const 65536))",
        "(global $__stack_pointer (export \"__stack_pointer\") i32 (i32.const 65536))",
    );

    assert_module_error(&wat, "__stack_pointer must be mutable");
}

#[test]
fn module_preflight_rejects_mistyped_stack_pointer() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        "(global $__stack_pointer (export \"__stack_pointer\") (mut i32) (i32.const 65536))",
        "(global $__stack_pointer (export \"__stack_pointer\") (mut i64) (i64.const 65536))",
    );

    assert_module_error(&wat, "__stack_pointer must be an i32 global");
}

#[test]
fn module_preflight_rejects_missing_required_function() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        "(export \"qjs_eval\")",
        "(export \"qjs_eval_missing\")",
    );

    assert_module_error(&wat, "does not export qjs_eval");
}

#[test]
fn module_preflight_rejects_mistyped_required_function() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        r#"  (func (export "qjs_eval")
    (param $code i32)
    (param $code_len i32)
    (param $filename i32)
    (param $flags i32)
    (result i32)
    i32.const 4)"#,
        r#"  (func (export "qjs_eval") (param $code i32) (result i32)
    i32.const 4)"#,
    );

    assert_module_error_contains(
        &wat,
        &[
            "export qjs_eval has signature (i32) -> (i32)",
            "expected (i32, i32, i32, i32) -> (i32)",
        ],
    );
}

#[test]
fn module_preflight_rejects_mistyped_f64_result_function() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        r#"  (func (export "qjs_get_float64") (param $value i32) (result f64)
    f64.const 0)"#,
        r#"  (func (export "qjs_get_float64") (param $value i32) (result i32)
    i32.const 0)"#,
    );

    assert_module_error_contains(
        &wat,
        &[
            "export qjs_get_float64 has signature (i32) -> (i32)",
            "expected (i32) -> (f64)",
        ],
    );
}

#[test]
fn module_preflight_rejects_unexpected_result_on_void_function() {
    let wat = replace_once(
        MINIMAL_ABI_WAT,
        r#"  (func (export "qjs_free_value") (param $value i32))"#,
        r#"  (func (export "qjs_free_value") (param $value i32) (result i32)
    i32.const 0)"#,
    );

    assert_module_error_contains(
        &wat,
        &[
            "export qjs_free_value has signature (i32) -> (i32)",
            "expected (i32) -> ()",
        ],
    );
}
