use super::super::super::guest_read::{quickjs_pending_exception_string, read_guest_u32};
use super::super::super::{HostState, finish_host_cleanup, host_import_error, quickjs_export};
use super::slice::read_array_buffer_slice_bytes;
use crate::QuickJsTypedArrayKind;
use crate::guest::guest_i32_add;
use wasmtime::{Caller, Memory};

pub(super) fn read_typed_array_bytes(
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

pub(super) fn read_data_view_bytes(
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
