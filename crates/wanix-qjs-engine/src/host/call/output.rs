use super::binary::quickjs_binary_value_to_js;
use super::{
    HostState, QuickJsCopiedValue, QuickJsValue, ensure_non_null_js_value, finish_host_cleanup,
    host_import_error, quickjs_export, write_guest_bytes,
};
use crate::guest::{host_len_i32, i64_to_guest_i32_halves};
use wasmtime::Caller;

pub(super) fn quickjs_callback_value_to_js(
    caller: &mut Caller<'_, HostState>,
    value: QuickJsCopiedValue,
) -> wasmtime::Result<i32> {
    match value {
        QuickJsCopiedValue::Scalar(value) => quickjs_scalar_value_to_js(caller, value),
        QuickJsCopiedValue::Binary(value) => quickjs_binary_value_to_js(caller, value),
    }
}

fn quickjs_scalar_value_to_js(
    caller: &mut Caller<'_, HostState>,
    value: QuickJsValue,
) -> wasmtime::Result<i32> {
    match value {
        QuickJsValue::Undefined => get_quickjs_singleton(caller, "qjs_get_undefined"),
        QuickJsValue::Null => get_quickjs_singleton(caller, "qjs_get_null"),
        QuickJsValue::Bool(value) => get_quickjs_bool(caller, value),
        QuickJsValue::Number(value) => create_quickjs_number(caller, value),
        QuickJsValue::String(value) => create_quickjs_string(caller, &value),
        QuickJsValue::BigIntI64(value) => create_quickjs_big_int64(caller, value),
    }
}

fn get_quickjs_singleton(
    caller: &mut Caller<'_, HostState>,
    export_name: &'static str,
) -> wasmtime::Result<i32> {
    let qjs_get_value = quickjs_export::<(), i32>(caller, export_name)?;
    ensure_non_null_js_value(qjs_get_value.call(caller, ())?, export_name)
}

fn get_quickjs_bool(caller: &mut Caller<'_, HostState>, value: bool) -> wasmtime::Result<i32> {
    let export_name = if value {
        "qjs_get_true"
    } else {
        "qjs_get_false"
    };
    get_quickjs_singleton(caller, export_name)
}

fn create_quickjs_number(caller: &mut Caller<'_, HostState>, value: f64) -> wasmtime::Result<i32> {
    let qjs_new_number = quickjs_export::<f64, i32>(caller, "qjs_new_number")?;
    ensure_non_null_js_value(qjs_new_number.call(caller, value)?, "qjs_new_number")
}

fn create_quickjs_big_int64(
    caller: &mut Caller<'_, HostState>,
    value: i64,
) -> wasmtime::Result<i32> {
    let (lo, hi) = i64_to_guest_i32_halves(value);
    let qjs_new_big_int64 = quickjs_export::<(i32, i32), i32>(caller, "qjs_new_big_int64")?;
    ensure_non_null_js_value(
        qjs_new_big_int64.call(caller, (lo, hi))?,
        "qjs_new_big_int64",
    )
}

fn create_quickjs_string(caller: &mut Caller<'_, HostState>, value: &str) -> wasmtime::Result<i32> {
    let bytes = value.as_bytes();
    let len = u32::try_from(bytes.len())
        .map_err(|_| wasmtime::Error::msg("host callback string is too large"))?;
    let len_i32 = host_len_i32(len)
        .ok_or_else(|| wasmtime::Error::msg("host callback string is too large"))?;
    let allocation_len = len.max(1);
    let allocation_len_i32 = host_len_i32(allocation_len)
        .ok_or_else(|| wasmtime::Error::msg("host callback string allocation is too large"))?;

    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let ptr = wasm_malloc.call(&mut *caller, allocation_len_i32)?;
    if ptr == 0 {
        return Err(host_import_error(
            "wasm_malloc returned null for host callback string",
        ));
    }

    let result = write_guest_bytes(caller, ptr, bytes).and_then(|()| {
        let qjs_new_string = quickjs_export::<(i32, i32), i32>(caller, "qjs_new_string")?;
        ensure_non_null_js_value(
            qjs_new_string.call(&mut *caller, (ptr, len_i32))?,
            "qjs_new_string",
        )
    });

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, ptr);
    finish_host_cleanup(result, cleanup)
}

pub(super) fn host_error_to_js_exception(
    caller: &mut Caller<'_, HostState>,
    err: wasmtime::Error,
) -> wasmtime::Result<i32> {
    let message = format!("Rust host callback failed: {err:#}");
    let message = create_quickjs_string(caller, &message)?;
    let result = quickjs_export::<i32, i32>(caller, "qjs_throw")
        .and_then(|qjs_throw| qjs_throw.call(&mut *caller, message))
        .and_then(|ptr| ensure_non_null_js_value(ptr, "qjs_throw"));
    let cleanup = quickjs_export::<i32, ()>(caller, "qjs_free_value")
        .and_then(|qjs_free_value| qjs_free_value.call(caller, message));
    finish_host_cleanup(result, cleanup)
}
