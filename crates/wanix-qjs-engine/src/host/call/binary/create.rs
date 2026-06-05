use super::super::{
    HostState, finish_host_cleanup, host_import_error, quickjs_export, write_guest_bytes,
};
use crate::guest::host_len_i32;
use crate::{QuickJsBinaryValue, QuickJsTypedArrayKind};
use wasmtime::Caller;

pub(in crate::host::call) fn quickjs_binary_value_to_js(
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
        super::super::ensure_non_null_js_value(
            create.call(&mut *caller, (ptr, len_i32))?,
            create_export,
        )
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
        super::super::ensure_non_null_js_value(
            create.call(&mut *caller, (kind.abi(), ptr, len_i32))?,
            "qjs_new_typed_array",
        )
    });

    let wasm_free = quickjs_export::<i32, ()>(caller, "wasm_free")?;
    let cleanup = wasm_free.call(caller, ptr);
    finish_host_cleanup(result, cleanup)
}
