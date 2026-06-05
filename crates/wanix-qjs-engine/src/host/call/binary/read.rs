use super::super::guest_read::{quickjs_pending_exception_string, read_guest_u32};
use super::super::scalar::maybe_quickjs_value_to_scalar;
use super::super::{
    HostState, finish_host_cleanup, host_import_error, optional_quickjs_export, quickjs_export,
};
use crate::allocation::try_copy_bytes;
mod slice;
mod view;
use crate::{QuickJsBinaryValue, QuickJsCopiedValue, QuickJsTypedArrayKind};
use slice::read_guest_bytes;
use view::{read_data_view_bytes, read_typed_array_bytes};
use wasmtime::{Caller, Memory, TypedFunc};

const QUICKJS_LEN_OUT_BYTES: i32 = 4;

pub(in crate::host::call) fn quickjs_value_to_callback(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<QuickJsCopiedValue> {
    if let Some(value) = read_scalar_callback_value(memory, caller, value)? {
        return Ok(value);
    }
    if let Some(value) = read_array_buffer_callback_value(memory, caller, value)? {
        return Ok(value.into());
    }
    if let Some(value) = read_uint8_array_callback_value(memory, caller, value)? {
        return Ok(value.into());
    }
    if let Some(value) = read_typed_array_callback_value(memory, caller, value)? {
        return Ok(value.into());
    }
    if let Some(value) = read_data_view_callback_value(memory, caller, value)? {
        return Ok(value.into());
    }

    Err(host_import_error("unsupported host callback argument type"))
}

fn read_scalar_callback_value(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsCopiedValue>> {
    maybe_quickjs_value_to_scalar(memory, caller, value).map(|value| value.map(Into::into))
}

fn read_array_buffer_callback_value(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsBinaryValue>> {
    if quickjs_binary_probe(caller, value, "qjs_is_array_buffer")? {
        return read_array_buffer_bytes(memory, caller, value)
            .map(QuickJsBinaryValue::ArrayBuffer)
            .map(Some);
    }
    Ok(None)
}

fn read_uint8_array_callback_value(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsBinaryValue>> {
    if quickjs_binary_probe(caller, value, "qjs_is_uint8_array")? {
        return read_uint8_array_bytes(memory, caller, value)
            .map(QuickJsBinaryValue::Uint8Array)
            .map(Some);
    }
    Ok(None)
}

fn read_typed_array_callback_value(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsBinaryValue>> {
    if let Some(qjs_get_typed_array_type) =
        optional_quickjs_export::<i32, i32>(caller, "qjs_get_typed_array_type")?
    {
        let kind_abi = qjs_get_typed_array_type.call(&mut *caller, value)?;
        if let Some(kind) = QuickJsTypedArrayKind::from_abi(kind_abi) {
            let bytes = read_typed_array_bytes(memory, caller, value, kind)?;
            return QuickJsBinaryValue::typed_array_from_bytes(kind, bytes)
                .map(Some)
                .map_err(|err| host_import_error(format!("{err:#}")));
        }
    }
    Ok(None)
}

fn read_data_view_callback_value(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
) -> wasmtime::Result<Option<QuickJsBinaryValue>> {
    if quickjs_binary_probe(caller, value, "qjs_is_data_view")? {
        return read_data_view_bytes(memory, caller, value)
            .map(QuickJsBinaryValue::DataView)
            .map(Some);
    }
    Ok(None)
}

fn quickjs_binary_probe(
    caller: &mut Caller<'_, HostState>,
    value: i32,
    export_name: &'static str,
) -> wasmtime::Result<bool> {
    Ok(optional_quickjs_export::<i32, i32>(caller, export_name)?
        .map(|probe| probe.call(&mut *caller, value).map(|matches| matches != 0))
        .transpose()?
        .unwrap_or(false))
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

fn read_binary_bytes_with_len_out(
    memory: &Memory,
    caller: &mut Caller<'_, HostState>,
    value: i32,
    get_bytes: &TypedFunc<(i32, i32), i32>,
    source: &'static str,
    label: &'static str,
) -> wasmtime::Result<Vec<u8>> {
    let wasm_malloc = quickjs_export::<i32, i32>(caller, "wasm_malloc")?;
    let len_out = wasm_malloc.call(&mut *caller, QUICKJS_LEN_OUT_BYTES)?;
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
