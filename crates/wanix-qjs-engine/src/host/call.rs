use super::guest_memory::{guest_len, guest_offset_at, guest_range};
use super::{HostCallbackMode, HostState, QuickJsCopiedValue, QuickJsValue, caller_memory};
use crate::allocation::{try_copy_bytes, try_copy_str};
use crate::guest::{
    guest_i32_add, guest_offset, host_len_i32, i64_from_guest_u32_halves, i64_to_guest_i32_halves,
};
use crate::{QuickJsBinaryValue, QuickJsTypedArrayKind};
use wasmtime::{Caller, Extern, Linker, Memory, TypedFunc, WasmParams, WasmResults};

mod scalar;
use scalar::{maybe_quickjs_value_to_scalar, quickjs_value_to_scalar};

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

fn quickjs_value_to_callback(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<QuickJsCopiedValue> {
    if let Some(value) = maybe_quickjs_value_to_scalar(memory, caller, value)? {
        return Ok(value.into());
    }

    if let Some(qjs_is_array_buffer) =
        optional_quickjs_export::<i32, i32>(caller, "qjs_is_array_buffer")?
        && qjs_is_array_buffer.call(&mut *caller, value)? != 0
    {
        return read_array_buffer_bytes(memory, caller, value)
            .map(QuickJsBinaryValue::ArrayBuffer)
            .map(Into::into);
    }

    if let Some(qjs_is_uint8_array) =
        optional_quickjs_export::<i32, i32>(caller, "qjs_is_uint8_array")?
        && qjs_is_uint8_array.call(&mut *caller, value)? != 0
    {
        return read_uint8_array_bytes(memory, caller, value)
            .map(QuickJsBinaryValue::Uint8Array)
            .map(Into::into);
    }

    if let Some(qjs_get_typed_array_type) =
        optional_quickjs_export::<i32, i32>(caller, "qjs_get_typed_array_type")?
    {
        let kind_abi = qjs_get_typed_array_type.call(&mut *caller, value)?;
        if let Some(kind) = QuickJsTypedArrayKind::from_abi(kind_abi) {
            let bytes = read_typed_array_bytes(memory, caller, value, kind)?;
            return QuickJsBinaryValue::typed_array_from_bytes(kind, bytes)
                .map(Into::into)
                .map_err(|err| host_import_error(format!("{err:#}")));
        }
    }

    if let Some(qjs_is_data_view) = optional_quickjs_export::<i32, i32>(caller, "qjs_is_data_view")?
        && qjs_is_data_view.call(&mut *caller, value)? != 0
    {
        return read_data_view_bytes(memory, caller, value)
            .map(QuickJsBinaryValue::DataView)
            .map(Into::into);
    }

    Err(host_import_error("unsupported host callback argument type"))
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

fn quickjs_binary_value_to_js(
    caller: &mut Caller<'_, HostState>,
    value: QuickJsBinaryValue,
) -> wasmtime::Result<i32> {
    match value {
        QuickJsBinaryValue::ArrayBuffer(bytes) => create_quickjs_binary(
            caller,
            &bytes,
            "qjs_new_array_buffer",
            "host callback ArrayBuffer",
        ),
        QuickJsBinaryValue::Uint8Array(bytes) => create_quickjs_binary(
            caller,
            &bytes,
            "qjs_new_uint8_array",
            "host callback Uint8Array",
        ),
        QuickJsBinaryValue::TypedArray { kind, bytes } => {
            create_quickjs_typed_binary(caller, kind, &bytes)
        }
        QuickJsBinaryValue::DataView(bytes) => create_quickjs_binary(
            caller,
            &bytes,
            "qjs_new_data_view",
            "host callback DataView",
        ),
    }
}

fn create_quickjs_binary(
    caller: &mut Caller<'_, HostState>,
    bytes: &[u8],
    create_export: &'static str,
    label: &'static str,
) -> wasmtime::Result<i32> {
    let len = u32::try_from(bytes.len())
        .map_err(|_| wasmtime::Error::msg(format!("{label} is too large")))?;
    let len_i32 =
        host_len_i32(len).ok_or_else(|| wasmtime::Error::msg(format!("{label} is too large")))?;
    let allocation_len = len.max(1);
    let allocation_len_i32 = host_len_i32(allocation_len)
        .ok_or_else(|| wasmtime::Error::msg(format!("{label} allocation is too large")))?;

    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let ptr = wasm_malloc.call(&mut *caller, allocation_len_i32)?;
    if ptr == 0 {
        return Err(host_import_error(format!(
            "wasm_malloc returned null for {label}"
        )));
    }

    let result = write_guest_bytes(caller, ptr, bytes).and_then(|()| {
        let create = quickjs_export::<(i32, i32), i32>(caller, create_export)?;
        ensure_non_null_js_value(create.call(&mut *caller, (ptr, len_i32))?, create_export)
    });

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, ptr);
    finish_host_cleanup(result, cleanup)
}

fn create_quickjs_typed_binary(
    caller: &mut Caller<'_, HostState>,
    kind: QuickJsTypedArrayKind,
    bytes: &[u8],
) -> wasmtime::Result<i32> {
    kind.validate_byte_len(bytes.len())
        .map_err(|err| host_import_error(format!("{err:#}")))?;
    if kind == QuickJsTypedArrayKind::Uint8 {
        return create_quickjs_binary(
            caller,
            bytes,
            "qjs_new_uint8_array",
            "host callback Uint8Array",
        );
    }

    let label = kind.js_name();
    let len = u32::try_from(bytes.len())
        .map_err(|_| wasmtime::Error::msg(format!("host callback {label} is too large")))?;
    let len_i32 =
        host_len_i32(len).ok_or_else(|| wasmtime::Error::msg(format!("{label} is too large")))?;
    let allocation_len = len.max(1);
    let allocation_len_i32 = host_len_i32(allocation_len).ok_or_else(|| {
        wasmtime::Error::msg(format!("host callback {label} allocation is too large"))
    })?;

    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let ptr = wasm_malloc.call(&mut *caller, allocation_len_i32)?;
    if ptr == 0 {
        return Err(host_import_error(format!(
            "wasm_malloc returned null for host callback {label}"
        )));
    }

    let result = write_guest_bytes(caller, ptr, bytes).and_then(|()| {
        let create = quickjs_export::<(i32, i32, i32), i32>(caller, "qjs_new_typed_array")?;
        ensure_non_null_js_value(
            create.call(&mut *caller, (kind.abi(), ptr, len_i32))?,
            "qjs_new_typed_array",
        )
    });

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, ptr);
    finish_host_cleanup(result, cleanup)
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

fn read_array_buffer_bytes(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Vec<u8>> {
    let qjs_get_array_buffer = quickjs_export::<(i32, i32), i32>(caller, "qjs_get_array_buffer")?;
    read_binary_bytes_with_len_out(
        memory,
        caller,
        value,
        &qjs_get_array_buffer,
        "qjs_get_array_buffer",
        "QuickJS ArrayBuffer",
    )
}

fn read_uint8_array_bytes(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Vec<u8>> {
    let qjs_get_uint8_array = quickjs_export::<(i32, i32), i32>(caller, "qjs_get_uint8_array")?;
    read_binary_bytes_with_len_out(
        memory,
        caller,
        value,
        &qjs_get_uint8_array,
        "qjs_get_uint8_array",
        "QuickJS Uint8Array",
    )
}

fn read_typed_array_bytes(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
    kind: QuickJsTypedArrayKind,
) -> wasmtime::Result<Vec<u8>> {
    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let meta_out = wasm_malloc.call(&mut *caller, 12)?;
    if meta_out == 0 {
        return Err(host_import_error(
            "wasm_malloc returned null for QuickJS typed array metadata",
        ));
    }
    let result = (|| {
        let byte_offset_out = meta_out;
        let byte_length_out = guest_i32_add(meta_out, 4)
            .ok_or_else(|| host_import_error("typed array metadata pointer overflowed"))?;
        let bytes_per_element_out = guest_i32_add(meta_out, 8)
            .ok_or_else(|| host_import_error("typed array metadata pointer overflowed"))?;

        read_typed_array_bytes_with_meta(
            memory,
            caller,
            value,
            kind,
            byte_offset_out,
            byte_length_out,
            bytes_per_element_out,
        )
    })();

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, meta_out);
    finish_host_cleanup(result, cleanup)
}

fn read_typed_array_bytes_with_meta(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
    kind: QuickJsTypedArrayKind,
    byte_offset_out: i32,
    byte_length_out: i32,
    bytes_per_element_out: i32,
) -> wasmtime::Result<Vec<u8>> {
    let qjs_get_typed_array_buffer =
        quickjs_export::<(i32, i32, i32, i32), i32>(caller, "qjs_get_typed_array_buffer")?;
    let array_buffer = qjs_get_typed_array_buffer.call(
        &mut *caller,
        (
            value,
            byte_offset_out,
            byte_length_out,
            bytes_per_element_out,
        ),
    )?;
    if array_buffer == 0 {
        let message = quickjs_pending_exception_string(memory, caller)
            .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
        return Err(host_import_error(format!(
            "qjs_get_typed_array_buffer failed: {message}"
        )));
    }

    let result = (|| {
        let qjs_is_exception = quickjs_export::<i32, i32>(caller, "qjs_is_exception")?;
        if qjs_is_exception.call(&mut *caller, array_buffer)? != 0 {
            let message = quickjs_pending_exception_string(memory, caller)
                .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
            return Err(host_import_error(format!(
                "qjs_get_typed_array_buffer failed: {message}"
            )));
        }

        let byte_offset = read_guest_u32(memory, caller, byte_offset_out, "typed array offset")?;
        let byte_length = read_guest_u32(memory, caller, byte_length_out, "typed array length")?;
        let bytes_per_element = read_guest_u32(
            memory,
            caller,
            bytes_per_element_out,
            "typed array bytes per element",
        )?;
        if usize::try_from(bytes_per_element).ok() != Some(kind.bytes_per_element()) {
            Err(host_import_error(format!(
                "QuickJS {} reported inconsistent element width {bytes_per_element}",
                kind.js_name()
            )))
        } else {
            read_array_buffer_slice_bytes(
                memory,
                caller,
                array_buffer,
                byte_offset,
                byte_length,
                kind.js_name(),
            )
        }
    })();

    let qjs_free_value = quickjs_export::<i32, ()>(caller, "qjs_free_value")?;
    let cleanup = qjs_free_value.call(caller, array_buffer);
    finish_host_cleanup(result, cleanup)
}

fn read_data_view_bytes(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Vec<u8>> {
    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let meta_out = wasm_malloc.call(&mut *caller, 8)?;
    if meta_out == 0 {
        return Err(host_import_error(
            "wasm_malloc returned null for QuickJS DataView metadata",
        ));
    }
    let result = (|| {
        let byte_offset_out = meta_out;
        let byte_length_out = guest_i32_add(meta_out, 4)
            .ok_or_else(|| host_import_error("DataView metadata pointer overflowed"))?;

        read_data_view_bytes_with_meta(memory, caller, value, byte_offset_out, byte_length_out)
    })();

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, meta_out);
    finish_host_cleanup(result, cleanup)
}

fn read_data_view_bytes_with_meta(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
    byte_offset_out: i32,
    byte_length_out: i32,
) -> wasmtime::Result<Vec<u8>> {
    let qjs_get_data_view_buffer =
        quickjs_export::<(i32, i32, i32), i32>(caller, "qjs_get_data_view_buffer")?;
    let array_buffer =
        qjs_get_data_view_buffer.call(&mut *caller, (value, byte_offset_out, byte_length_out))?;
    if array_buffer == 0 {
        let message = quickjs_pending_exception_string(memory, caller)
            .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
        return Err(host_import_error(format!(
            "qjs_get_data_view_buffer failed: {message}"
        )));
    }

    let result = (|| {
        let qjs_is_exception = quickjs_export::<i32, i32>(caller, "qjs_is_exception")?;
        if qjs_is_exception.call(&mut *caller, array_buffer)? != 0 {
            let message = quickjs_pending_exception_string(memory, caller)
                .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
            return Err(host_import_error(format!(
                "qjs_get_data_view_buffer failed: {message}"
            )));
        }

        let byte_offset = read_guest_u32(memory, caller, byte_offset_out, "DataView offset")?;
        let byte_length = read_guest_u32(memory, caller, byte_length_out, "DataView length")?;
        read_array_buffer_slice_bytes(
            memory,
            caller,
            array_buffer,
            byte_offset,
            byte_length,
            "DataView",
        )
    })();

    let qjs_free_value = quickjs_export::<i32, ()>(caller, "qjs_free_value")?;
    let cleanup = qjs_free_value.call(caller, array_buffer);
    finish_host_cleanup(result, cleanup)
}

fn read_binary_bytes_with_len_out(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
    get_bytes: &TypedFunc<(i32, i32), i32>,
    source: &'static str,
    label: &'static str,
) -> wasmtime::Result<Vec<u8>> {
    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let len_out = wasm_malloc.call(&mut *caller, 4)?;
    if len_out == 0 {
        return Err(host_import_error(format!(
            "wasm_malloc returned null for {label} length"
        )));
    }

    let result = match get_bytes.call(&mut *caller, (value, len_out)) {
        Ok(data_ptr) => {
            if data_ptr == 0 {
                let message = quickjs_pending_exception_string(memory, caller)
                    .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
                Err(host_import_error(format!("{source} failed: {message}")))
            } else {
                read_guest_u32(memory, caller, len_out, &format!("{label} length")).and_then(
                    |len| match len {
                        0 => try_copy_bytes(&[], label)
                            .map_err(|err| host_import_error(format!("{err:#}"))),
                        len => read_guest_bytes(memory, caller, data_ptr, len, label),
                    },
                )
            }
        }
        Err(err) => Err(err),
    };

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, len_out);
    finish_host_cleanup(result, cleanup)
}

fn read_array_buffer_slice_bytes(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
    byte_offset: u32,
    byte_length: u32,
    label: &'static str,
) -> wasmtime::Result<Vec<u8>> {
    let qjs_get_array_buffer = quickjs_export::<(i32, i32), i32>(caller, "qjs_get_array_buffer")?;
    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let len_out = wasm_malloc.call(&mut *caller, 4)?;
    if len_out == 0 {
        return Err(host_import_error(format!(
            "wasm_malloc returned null for {label} backing ArrayBuffer length"
        )));
    }

    let result = (|| match qjs_get_array_buffer.call(&mut *caller, (value, len_out)) {
        Ok(data_ptr) => {
            if data_ptr == 0 {
                let message = quickjs_pending_exception_string(memory, caller)
                    .unwrap_or_else(|err| format!("failed to take QuickJS exception: {err:#}"));
                Err(host_import_error(format!(
                    "qjs_get_array_buffer failed: {message}"
                )))
            } else {
                let buffer_len =
                    read_guest_u32(memory, caller, len_out, "backing ArrayBuffer length")?;
                let end = byte_offset
                    .checked_add(byte_length)
                    .ok_or_else(|| host_import_error(format!("{label} byte range overflowed")))?;
                if end > buffer_len {
                    return Err(host_import_error(format!(
                        "{label} byte range [{byte_offset}, {end}) exceeds backing ArrayBuffer length {buffer_len}"
                    )));
                }
                let start = guest_offset(data_ptr)
                    .checked_add(usize::try_from(byte_offset).map_err(|err| {
                        host_import_error(format!("{label} byte offset is too large: {err}"))
                    })?)
                    .ok_or_else(|| {
                        host_import_error(format!("{label} pointer offset overflowed"))
                    })?;
                let len = usize::try_from(byte_length).map_err(|err| {
                    host_import_error(format!("{label} byte length is too large: {err}"))
                })?;
                let range = guest_range(memory, caller, start, len)?;
                try_copy_bytes(&memory.data(&*caller)[range], label)
                    .map_err(|err| host_import_error(format!("{err:#}")))
            }
        }
        Err(err) => Err(err),
    })();

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, len_out);
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

fn quickjs_value_to_string(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<String> {
    let qjs_get_string = quickjs_export::<i32, i32>(caller, "qjs_get_string")?;
    let c_string = qjs_get_string.call(&mut *caller, value)?;
    if c_string == 0 {
        return Err(host_import_error(
            "qjs_get_string returned a null C string pointer",
        ));
    }
    read_and_free_quickjs_c_string(memory, caller, c_string)
}

fn quickjs_pending_exception_string(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
) -> wasmtime::Result<String> {
    let qjs_get_exception = quickjs_export::<(), i32>(caller, "qjs_get_exception")?;
    let exception = qjs_get_exception.call(&mut *caller, ())?;
    if exception == 0 {
        return Err(host_import_error(
            "qjs_get_exception returned a null JSValue pointer",
        ));
    }
    let result = quickjs_value_to_string(memory, caller, exception);
    let qjs_free_value = quickjs_export::<i32, ()>(caller, "qjs_free_value")?;
    let cleanup = qjs_free_value.call(caller, exception);
    finish_host_cleanup(result, cleanup)
}

fn read_and_free_quickjs_c_string(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
) -> wasmtime::Result<String> {
    let string = read_guest_c_string(memory, caller, ptr);
    let qjs_free_cstring = quickjs_export::<i32, ()>(caller, "qjs_free_cstring")?;
    let cleanup = qjs_free_cstring.call(caller, ptr);
    finish_host_cleanup(string, cleanup)
}

fn read_guest_c_string(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: i32,
) -> wasmtime::Result<String> {
    if ptr == 0 {
        return Err(host_import_error("null guest C string pointer"));
    }
    let data = memory.data(caller);
    let start = guest_offset(ptr);
    if start >= data.len() {
        return Err(host_import_error(
            "guest C string pointer is outside memory",
        ));
    }
    let mut end = start;
    while end < data.len() && data[end] != 0 {
        end += 1;
    }
    if end == data.len() {
        return Err(host_import_error("unterminated guest C string"));
    }
    read_guest_utf8(memory, caller, start, end - start, "guest C string")
}

fn read_guest_bytes(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: i32,
    len: u32,
    label: &str,
) -> wasmtime::Result<Vec<u8>> {
    let len = usize::try_from(len)
        .map_err(|err| host_import_error(format!("{label} length is too large: {err}")))?;
    let range = guest_range(memory, caller, guest_offset(ptr), len)?;
    try_copy_bytes(&memory.data(caller)[range], label)
        .map_err(|err| host_import_error(format!("{err:#}")))
}

fn read_guest_u32(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: i32,
    _label: &str,
) -> wasmtime::Result<u32> {
    let range = guest_range(memory, caller, guest_offset(ptr), 4)?;
    let mut bytes = [0; 4];
    bytes.copy_from_slice(&memory.data(caller)[range]);
    Ok(u32::from_le_bytes(bytes))
}

fn read_guest_utf8(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: usize,
    len: usize,
    label: &str,
) -> wasmtime::Result<String> {
    let range = guest_range(memory, caller, ptr, len)?;
    let bytes = &memory.data(caller)[range];
    let value = std::str::from_utf8(bytes)
        .map_err(|err| wasmtime::Error::msg(format!("{label} was not valid UTF-8: {err}")))?;
    try_copy_str(value, label).map_err(|err| wasmtime::Error::msg(format!("{err:#}")))
}

fn read_guest_i32(
    memory: &Memory,
    caller: &Caller<'_, HostState>,
    ptr: usize,
) -> wasmtime::Result<i32> {
    let range = guest_range(memory, caller, ptr, JS_VALUE_PTR_LEN)?;
    let mut bytes = [0; JS_VALUE_PTR_LEN];
    bytes.copy_from_slice(&memory.data(caller)[range]);
    Ok(i32::from_le_bytes(bytes))
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
