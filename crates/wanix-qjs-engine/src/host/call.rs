use super::guest_memory::{guest_len, guest_offset_at};
use super::{HostCallbackMode, HostState, QuickJsCopiedValue, QuickJsValue, caller_memory};
use crate::guest::{
    guest_i32_add, guest_offset, host_len_i32, i64_from_guest_u32_halves, i64_to_guest_i32_halves,
};
use wasmtime::{Caller, Extern, Linker, Memory, TypedFunc, WasmParams, WasmResults};

mod binary;
mod guest_read;
mod scalar;
use binary::{quickjs_binary_value_to_js, quickjs_value_to_callback};
use guest_read::{
    quickjs_pending_exception_string, read_guest_i32, read_guest_u32, read_guest_utf8,
};
use scalar::quickjs_value_to_scalar;

const JS_VALUE_PTR_LEN: usize = 4;

pub(super) fn define_import(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "env",
        "host_call",
        |mut caller: Caller<'_, HostState>,
         name_ptr: i32,
         name_len: i32,
         _this_value: i32,
         argc: i32,
         argv: i32|
         -> wasmtime::Result<i32> {
            if caller.data().memory().is_none() {
                return Ok(0);
            }
            match dispatch_host_call(&mut caller, name_ptr, name_len, argc, argv) {
                Ok(value) => Ok(value),
                Err(err) => host_error_to_js_exception(&mut caller, err),
            }
        },
    )?;
    Ok(())
}

fn dispatch_host_call(
    caller: &mut Caller<'_, HostState>,
    name_ptr: i32,
    name_len: i32,
    argc: i32,
    argv: i32,
) -> wasmtime::Result<i32> {
    let memory = caller_memory(caller)?;
    let name = read_guest_utf8(
        &memory,
        caller,
        guest_offset(name_ptr),
        guest_len(name_len)?,
        "host callback name",
    )?;
    let mode = caller
        .data()
        .host_callback_mode(&name)
        .map_err(|err| host_import_error(format!("{err:#}")))?;
    let args = read_host_callback_args(&memory, caller, argc, argv, mode)?;
    let value = caller
        .data_mut()
        .call_host_callback(&name, &args)
        .map_err(|err| host_import_error(format!("{err:#}")))?;
    quickjs_callback_value_to_js(caller, value)
}

fn read_host_callback_args(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    argc: i32,
    argv: i32,
    mode: HostCallbackMode,
) -> wasmtime::Result<Vec<QuickJsCopiedValue>> {
    let argc = guest_len(argc)?;
    let mut args = Vec::new();
    args.try_reserve_exact(argc).map_err(|err| {
        wasmtime::Error::msg(format!("host callback args allocation failed: {err}"))
    })?;
    if argc == 0 {
        return Ok(args);
    }
    if argv == 0 {
        return Err(host_import_error(format!(
            "host callback argv pointer is null for {argc} arguments"
        )));
    }
    for index in 0..argc {
        let ptr = read_guest_i32(
            memory,
            caller,
            guest_offset_at(argv, index, JS_VALUE_PTR_LEN)?,
        )?;
        let arg = match mode {
            HostCallbackMode::Scalar => quickjs_value_to_scalar(memory, caller, ptr)?.into(),
            HostCallbackMode::BinaryCapable => quickjs_value_to_callback(memory, caller, ptr)?,
        };
        args.push(arg);
    }
    Ok(args)
}

fn quickjs_callback_value_to_js(
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
        QuickJsValue::Undefined => {
            let qjs_get_undefined = quickjs_export::<(), i32>(caller, "qjs_get_undefined")?;
            ensure_non_null_js_value(qjs_get_undefined.call(caller, ())?, "qjs_get_undefined")
        }
        QuickJsValue::Null => {
            let qjs_get_null = quickjs_export::<(), i32>(caller, "qjs_get_null")?;
            ensure_non_null_js_value(qjs_get_null.call(caller, ())?, "qjs_get_null")
        }
        QuickJsValue::Bool(value) => {
            let name = if value {
                "qjs_get_true"
            } else {
                "qjs_get_false"
            };
            let qjs_get_bool = quickjs_export::<(), i32>(caller, name)?;
            ensure_non_null_js_value(qjs_get_bool.call(caller, ())?, name)
        }
        QuickJsValue::Number(value) => {
            let qjs_new_number = quickjs_export::<f64, i32>(caller, "qjs_new_number")?;
            ensure_non_null_js_value(qjs_new_number.call(caller, value)?, "qjs_new_number")
        }
        QuickJsValue::String(value) => create_quickjs_string(caller, &value),
        QuickJsValue::BigIntI64(value) => create_quickjs_big_int64(caller, value),
    }
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

fn write_guest_bytes(
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
    bytes: &[u8],
) -> wasmtime::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let memory = caller_memory(caller)?;
    memory.write(caller, guest_offset(ptr), bytes)?;
    Ok(())
}

fn host_error_to_js_exception(
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

fn read_big_int64_value(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<i64> {
    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let out = wasm_malloc.call(&mut *caller, 8)?;
    if out == 0 {
        return Err(host_import_error(
            "wasm_malloc returned null for QuickJS BigInt output words",
        ));
    }

    let result = (|| {
        let hi_out = guest_i32_add(out, 4)
            .ok_or_else(|| host_import_error("BigInt output pointer overflowed"))?;
        let qjs_get_big_int64 =
            quickjs_export::<(i32, i32, i32), i32>(caller, "qjs_get_big_int64")?;
        let ret = qjs_get_big_int64.call(&mut *caller, (value, out, hi_out))?;
        if ret != 0 {
            let message = quickjs_pending_exception_string(memory, caller)
                .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
            return Err(host_import_error(format!(
                "qjs_get_big_int64 failed: {message}"
            )));
        }
        let lo = read_guest_u32(memory, caller, out, "BigInt low word")?;
        let hi = read_guest_u32(memory, caller, hi_out, "BigInt high word")?;
        Ok(i64_from_guest_u32_halves(lo, hi))
    })();

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, out);
    finish_host_cleanup(result, cleanup)
}

fn finish_host_cleanup<T>(
    result: wasmtime::Result<T>,
    cleanup: wasmtime::Result<()>,
) -> wasmtime::Result<T> {
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(err), Ok(())) => Err(err),
        (Ok(_value), Err(cleanup_err)) => Err(wasmtime::Error::msg(format!(
            "host cleanup failed after successful result: {cleanup_err:#}"
        ))),
        (Err(err), Err(cleanup_err)) => Err(wasmtime::Error::msg(format!(
            "{err:#}; additionally, host cleanup failed: {cleanup_err:#}"
        ))),
    }
}

fn quickjs_export<P, R>(
    caller: &mut Caller<'_, HostState>,
    name: &str,
) -> wasmtime::Result<TypedFunc<P, R>>
where
    P: WasmParams,
    R: WasmResults,
{
    let Some(Extern::Func(func)) = caller.get_export(name) else {
        return Err(host_import_error(format!(
            "QuickJS WASM module does not export {name}"
        )));
    };
    func.typed::<P, R>(&*caller)
        .map_err(|err| wasmtime::Error::msg(format!("missing or mistyped {name} export: {err:#}")))
}

fn optional_quickjs_export<P, R>(
    caller: &mut Caller<'_, HostState>,
    name: &str,
) -> wasmtime::Result<Option<TypedFunc<P, R>>>
where
    P: WasmParams,
    R: WasmResults,
{
    let Some(export) = caller.get_export(name) else {
        return Ok(None);
    };
    let Extern::Func(func) = export else {
        return Err(host_import_error(format!(
            "missing or mistyped {name} export: export is not a function"
        )));
    };
    func.typed::<P, R>(&*caller)
        .map(Some)
        .map_err(|err| wasmtime::Error::msg(format!("missing or mistyped {name} export: {err:#}")))
}

fn ensure_non_null_js_value(ptr: i32, source: &str) -> wasmtime::Result<i32> {
    if ptr == 0 {
        return Err(host_import_error(format!(
            "{source} returned a null JSValue pointer"
        )));
    }
    Ok(ptr)
}

fn host_import_error(message: impl Into<String>) -> wasmtime::Error {
    wasmtime::Error::msg(message.into())
}
