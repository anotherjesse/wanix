use super::super::super::guest_read::{quickjs_pending_exception_string, read_guest_u32};
use super::super::super::{HostState, finish_host_cleanup, host_import_error, quickjs_export};
use crate::allocation::try_copy_bytes;
use crate::guest::guest_offset;
use crate::host::guest_memory::guest_range;
use wasmtime::{Caller, Memory};

pub(super) fn read_array_buffer_slice_bytes(
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

pub(super) fn read_guest_bytes(
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
